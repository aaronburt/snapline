# SnapLine (Tauri edition): from-scratch build plan

Written: 2026-11-09. Owner: Aaron.

This file is a complete, self-contained plan for building SnapLine for Windows from an empty folder, using Rust for everything native and Tauri 2 (WebView2) for the normal windows. It replaces the Slint UI layer of the original plan. Product scope, features, data formats and the v2 design are unchanged.

It is written for an AI coding agent (or several running in parallel) that has **no other context**. Everything the agent needs is in this file, in `design/v2/` (the v2 design source files) and in `design/tokens.toml`. If something is not written here, it has not been decided.

Sources this plan was built from (for humans; the agent does not need to open them):

- Build plan and engineering spec: https://claude.ai/artifact/JWHssZ8o9UK8nVPptdD5oS
- v2 design canvas: https://claude.ai/artifact/R6eyMfsiAKdYydfUKTRwnw (copied into `design/v2/`)
- The Slint build's `progress.md` (milestones 0 to 6 built with Slint, measured numbers, lessons)

---

## 0. Rules for the agent executing this plan

Read this section before every task.

1. **Do not assume.** If a value, label, size, colour, file name, behaviour or library is not stated in this file or in `design/v2/`, stop and ask Aaron. Do not invent a default, do not pick a library, do not "improve" the design. Write the question down in the task's notes and move to a task that is not blocked.
2. **Do exactly the task.** Only edit files your task **owns**, plus the **shared** edits it lists. If you need to change another file, stop and ask the integrator.
3. **No code comments, anywhere.** Not in Rust, TypeScript, Svelte, CSS, HTML, TOML, SQL or build scripts. The only exceptions are `// SAFETY:` lines directly above `unsafe` blocks in Rust, and crate docs written as `#![doc = "..."]` attributes (not `//!`). `cargo xtask check` enforces this (task 0.2).
4. **Text in the UI is copied exactly** from this file or from `design/v2/`. Spelling is UK English ("Colour", "Minimise", "Organise"). The product name shown in the UI is **Snapline** (lower-case L), as in the design. Code, crate names and file paths use `snapline`.
5. **Never add a dependency** that is not listed in section 5.9. Never upgrade a dependency mid-milestone. If you think one is needed, ask.
6. **Never put anything on Aaron's screen without asking.** That includes launching the app, opening windows, running `cargo xtask budget`, running tests that create windows, touch the clipboard, register hotkeys, or take screenshots. Headless tests are fine.
7. **Commits:** a milestone lands on `main` as exactly one commit, made only after `cargo xtask ci` passes. Task branches are squashed by the integrator (section 9). Commit messages follow Conventional Commits (`feat(upload): add S3 SigV4`).
8. **Secrets** (API keys, tokens, passwords, header values marked secret) never go in a file, a log, an IPC response, a test snapshot or an error message.
9. When a task says **"Done when"**, all of it must be true, and you must say how you checked each point.

---

## 1. What SnapLine is

SnapLine is a ShareX alternative for Windows: a tray app that captures screenshots and screen recordings, lets you annotate them, saves them, uploads them to image hosts or your own server, and copies a link. It is open source under GPLv3.

"Lean" means **low RAM and low CPU**, especially while idle in the tray. Disk size does not matter.

The everyday loop is: press PrtSc, drag a region, (optionally annotate), and a link is on the clipboard.

---

## 2. Decisions already made

| Area | Decision |
|------|----------|
| Language | Rust (stable, MSVC target) for all native code |
| Normal windows (quick panel, main window, settings) | Tauri 2 with WebView2, HTML/CSS/TypeScript with Svelte 5 |
| Capture overlay, annotation toolbar, pin, ruler, colour picker, recording border | Native Win32 + Direct2D + DirectComposition + DirectWrite. **Never a webview.** |
| Design | The v2 boards in `design/v2/` are the spec. v1 boards are not included and are never built. |
| App views | Only History, Workflows and Tools in the main window navigation, plus Settings |
| Platforms | Windows first. Minimum Windows 10 22H2; every Windows 11 release is supported. Linux is a later, separate build. |
| Licence | GPLv3-or-later. All Rust and npm dependencies must be GPLv3-compatible. |
| FFmpeg | A pinned GPL build of `ffmpeg.exe` ships inside the install folder as a separate binary, with its licence and source link |
| OCR | Built-in Windows OCR (`Windows.Media.Ocr`) |
| Secrets | Windows Credential Manager through the Win32 API |
| Hotkey clashes | Fail closed: if two things want the same chord, neither is bound, and the user is told |
| Repository | Local git repo, no remote, branch `main` |
| AVIF | Deferred. Not built, and hidden in the UI. |
| Code signing, auto-update | Deferred until there is a public remote |
| App identifier | `io.github.aaronburt.snapline` (Tauri identifier and Windows AppUserModelID) |

---

## 3. Why Tauri, and what it costs

Aaron asked for this plan with Tauri instead of Slint. The honest trade-off:

**What Tauri gives:** HTML/CSS make the v2 design (which was drawn in HTML) much faster to reproduce exactly. Far more people and tools know web UI. Layout, text, accessibility and theming come from a mature engine.

**What it costs:** each WebView2 window runs in a family of Microsoft Edge processes (browser, GPU, renderer, utility, crashpad). Measured on this PC on 2026-11-09 with an isolated Tauri 2.12.5 release prototype:

| State | Measured |
|-------|----------|
| Tray idle, no window opened yet | 1.6 MiB private working set, 0 WebView2 processes |
| History window open (synthetic data) | 77.1 MiB across the app and 6 WebView2 processes |
| Tray idle after closing the window | **Not measured yet.** Task 1.1 measures it. |

For comparison, the Slint build idles at about 5.4 MB and its budget for an open window was 60 MB. A webview window cannot meet 60 MB, so section 6 sets new budgets for open windows. Idle budgets do not change.

Other costs: opening a window cold takes a few hundred milliseconds while WebView2 starts; Tauri brings in its own async runtime (tokio) and an IPC layer; there are two UI technologies (web for windows, Direct2D for the overlay).

**How this plan keeps it lean:**

1. The Rust core runs alone in the tray. **No webview exists at startup.** `app.windows` in `tauri.conf.json` is empty.
2. A webview is created only when the user opens the quick panel or the main window, and **destroyed** (not hidden) when it closes.
3. After the last webview closes, the app waits a short "linger" period (15 seconds) so reopening is fast, then makes sure every WebView2 process has exited.
4. Settings is a page inside the main window, not a second window, so at most two webviews ever exist (quick panel and main window).
5. Everything time-critical (PrtSc to overlay, annotation, recording) is native and never waits for a webview.
6. Images never travel through IPC as JSON or base64. Thumbnails are served to the webview by a custom URI scheme reading files from disk.
7. `cargo xtask budget` measures the app **plus all its WebView2 child processes**, so the numbers cannot hide.

Task 1.1 is a **gate**: if WebView2 processes do not exit after the last window is destroyed, work stops and Aaron chooses between the fallbacks in section 5.2.

---

## 4. Glossary

| Term | Meaning |
|------|---------|
| Core | The Rust code that always runs: tray, hotkeys, workers, overlay. Lives in the `snapline` process. |
| Webview | A Tauri `WebviewWindow` backed by WebView2. Only for the quick panel and the main window. |
| Quick panel | The small flyout above the tray icon (left-click). Board `Main-v2.dc.html`. |
| Main window | One window holding History, Workflows, Tools and Settings. Boards `App-v2*.dc.html` and `Settings-*-v2.dc.html`. |
| Overlay | The full-screen native capture UI with region selection and annotation toolbar. Board `Overlay.dc.html`. |
| Area | A feature group in the IPC contract: app, settings, hotkeys, history, upload, workflow, record, tool. |
| Integrator | The person or agent who creates milestone branches, merges task branches, runs `cargo xtask ci` and makes the milestone commit. |
| Private working set | What Task Manager's "Memory" column shows. Not "private bytes" or "commit size". |

---

## 5. Architecture

### 5.1 Processes and threads

One process, `snapline.exe`, plus WebView2 processes only while a webview exists, plus `ffmpeg.exe` only while recording or converting.

| Thread | Lifetime | Owns | Talks through |
|--------|----------|------|---------------|
| Main (Tauri event loop) | Always | Tauri app, tray icon and menu, webview windows, IPC command dispatch, custom URI scheme | Receives `AppEvent` (forwards to webviews), sends `Job` |
| Input | Always | A message-only window, `RegisterHotKey` for every bound chord, `WM_HOTKEY`, `WM_POWERBROADCAST` (resume), `WM_SETTINGCHANGE` (theme) | Sends `Job::Action(ActionId)`; receives `InputCmd` (rebind, pause) through a channel plus `PostThreadMessageW` wake-up |
| Worker | Always, blocked on its queue | Workflow runner, encoding, saving, OCR, thumbnails, the only history write connection | Receives `Job`, sends `AppEvent` and `UploadJob` |
| Uploader | Always, blocked on its queue | Upload queue, retries with backoff, timeouts | Receives `UploadJob`, sends `AppEvent` |
| Overlay | Only while an overlay, pin, ruler or picker is on screen | Overlay windows, Direct2D device, overlay state machine | Sends `OverlayResult` back to the worker |
| Recorder | Only while recording | FFmpeg child, WASAPI audio capture, recording border | Receives `RecordCmd`, sends `AppEvent` |
| Log writer | Always | Rolling log file (`tracing-appender` non-blocking) | Log channel |
| Tauri async runtime | Created by Tauri | Runs `async` IPC commands | Calls services through `spawn_blocking` |

Rules:

- When idle, every thread is blocked waiting for a message. No timers, no polling. The only timer allowed is the one-shot linger timer after the last webview closes (section 5.2) and the upload retry backoff.
- Threads talk over `crossbeam-channel`. No shared locks on hot paths.
- Background work never touches UI objects. Results go to the main thread as `AppEvent`, and the main thread forwards them to any open webview with `emit`.
- Rust is the single source of truth. A webview holds no state that matters: when it opens, it asks for everything it shows; when it closes, nothing is lost.

A PrtSc press flows like this: `WM_HOTKEY` on the input thread → `Job::Action("capture_region")` to the worker → worker starts the overlay thread → overlay returns the selection (and annotations, if the workflow has an annotate step) → worker runs the remaining workflow steps → an upload step becomes an `UploadJob` → the uploader finishes and sends `AppEvent::Upload(Uploaded)` → the main thread updates the tray tooltip and forwards the event to open webviews.

### 5.2 Webview lifecycle

The main thread owns a `WindowManager` (in `apps/snapline/src/windows.rs`) with two window kinds:

| Kind | Tauri label | URL | Created | Destroyed |
|------|-------------|-----|---------|-----------|
| Quick panel | `panel` | `index.html#/panel` | Tray left-click | When it loses focus, on Esc, or when an action in it opens something else |
| Main window | `main` | `index.html#/history` (or the route asked for) | Tray menu "Open Snapline" or "Settings", quick panel buttons, second launch of the exe, a notification click | When the user closes it (caption button, Alt+F4) |

How a window opens:

1. If a window of that kind exists, focus it and navigate it to the requested route by emitting `AppEvent::App(Navigate { route })`. Done.
2. Otherwise build it with `WebviewWindowBuilder` with `visible(false)`, the background colour of the current theme (`--mica` for main, `--layer` for panel) so there is no white flash, `decorations(false)`, the sizes in section 7, and the **same** `additional_browser_args` and data directory for every window (different values would start a second WebView2 browser process).
3. The page calls `ipc_app` → `Ready` once its first data is rendered. Only then does Rust call `show()` and `set_focus()`.
4. Cancel the linger timer if it is running.

How a window closes:

1. On `WindowEvent::CloseRequested` or a close action, call `window.destroy()`. Never `hide()`.
2. If no webview window remains, start the linger timer (15 seconds, a constant `LINGER`, not a user setting).
3. When the timer fires and still no webview exists, the app must hold no reference to the WebView2 environment, so all WebView2 processes exit.
4. `RunEvent::ExitRequested` is prevented unless the user chose Exit, so closing the last window never quits the app.

WebView2 settings for every window:

- Data directory: `%LOCALAPPDATA%\SnapLine\WebView2`.
- `additional_browser_args`: start from Tauri's own default (`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`), because setting this value replaces Tauri's default. Task 1.1 measures whether `--disable-background-networking` and `--disable-component-update` reduce memory without breaking anything, and keeps only those that do. The final list is recorded in ADR 0004.
- In release builds: no devtools, browser accelerator keys off (`ICoreWebView2Settings3::put_AreBrowserAcceleratorKeysEnabled(false)`, so F5, Ctrl+R, Ctrl+F, Ctrl+P do nothing), zoom off, the default context menu off except inside text inputs.
- Optional, measured in task 1.1: when the main window is minimised, set `ICoreWebView2_19::put_MemoryUsageTargetLevel(LOW)`, and back to `NORMAL` when restored. Keep it only if it saves at least 10 MB.

**Gate (task 1.1):** open the main window, close it, wait for the linger period plus 10 seconds, then count `msedgewebview2.exe` processes whose parent chain leads to `snapline.exe` and read the app's private working set. Pass when there are 0 such processes and the app is within 3 MB of its private working set before the window first opened. If it fails, stop and tell Aaron. The fallbacks, for Aaron to choose:

- **Fallback A, explicit teardown:** drop every Tauri webview handle, close each `ICoreWebView2Controller`, and release the environment through `with_webview`. Try this first; it changes little.
- **Fallback B, UI host process:** the core runs without Tauri (tray and menu through `Shell_NotifyIcon` and `TrackPopupMenuEx` in `platform-windows`), and the Tauri UI runs in a second process `snapline-ui.exe`, started on demand and exiting when its last window closes. The two talk over a named pipe (`\\.\pipe\snapline-<user SID>`) with the same request and event types as section 5.5, serialised as length-prefixed JSON. Memory always returns because the process exits. Costs about 100 ms more to open a window and a proxy layer. This is a different architecture and needs ADR 0005 and Aaron's approval before any code.

### 5.3 Repository layout

```
snapline/
  Cargo.toml                  workspace members, all shared dependency versions, lints, profiles
  Cargo.lock
  rust-toolchain.toml         stable, x86_64-pc-windows-msvc
  .cargo/config.toml          +crt-static, `xtask` alias
  deny.toml                   cargo-deny: GPLv3-compatible licences, advisories, bans
  .gitignore                  target/, ui/node_modules/, ui/dist/, third_party/
  LICENSE                     GPLv3 text
  README.md
  progress-tauri.md           this file
  design/
    tokens.toml               colour, type and radius tokens (source of truth)
    v2/                       the v2 design boards (*.dc.html) and canvas.json, read-only reference
  docs/adr/                   decision records 0001 onwards
  crates/
    core/                     snapline-core: domain types, pure logic, port traits. No OS calls, no unsafe.
    store/                    snapline-store: settings.toml and history.db
    upload/                   snapline-upload: HTTP, queue, uploaders
    platform-windows/         snapline-win: Win32/WinRT adapters for core's ports. Unsafe allowed.
    overlay-windows/          snapline-overlay: native overlay, toolbar, pin, ruler, picker, record border. Unsafe allowed.
    services/                 snapline-services: frontend-neutral application layer, threads, jobs, events, DTOs
  apps/
    snapline/                 the Tauri binary: wiring, tray, window manager, IPC command wrappers
      Cargo.toml
      build.rs                tauri-build, manifest, icon
      tauri.conf.json
      capabilities/main.json
      icons/
      src/
  ui/                         the web frontend (Svelte 5 + TypeScript + Vite)
    package.json
    package-lock.json
    vite.config.ts
    svelte.config.js
    tsconfig.json
    index.html
    src/
  xtask/                      build tasks in Rust (no shell scripts)
  assets/                     app icon, sounds, licence texts
  third_party/ffmpeg/         git-ignored, filled by `cargo xtask fetch-ffmpeg`
```

Dependencies point inward only: `apps/snapline` → `services` → `core`, `store`, `upload`, plus the port implementations from `platform-windows` and `overlay-windows` passed in at startup. `core` depends on nothing OS-specific. `services` never depends on `tauri`. `store` and `upload` never depend on `platform-windows`.

`#![forbid(unsafe_code)]` in `core`, `store`, `upload`, `services` and `apps/snapline`. Unsafe code lives only in `platform-windows` and `overlay-windows`, each FFI call in a small safe wrapper with a `// SAFETY:` line, and every handle (`HWND`, `HICON`, COM pointers, credential buffers) owned by a type that releases it in `Drop`.

### 5.4 Crate contracts

**`snapline-core`**: ids and newtypes (`CaptureId`, `DestId`, `WorkflowId`, `ActionId`, `Px`, `Chord`), geometry (`Point`, `Rect`, `Size`, DPI scale), `Image` (BGRA8 buffer with width, height, stride), port traits (below), the action catalogue, chord parsing and normalising, the hotkey plan, file naming tokens, link formats, the overlay state machine, the annotation model and commands, the software rasteriser, the workflow model and runner, presets, the recording state machine, FFmpeg argument builders, scroll stitching, ruler maths, colour conversions, QR wrappers, combiner layout.

Port traits (in `crates/core/src/ports.rs`):

```rust
pub trait ScreenCapture { fn monitors(&self) -> Vec<Monitor>; fn freeze_all(&self) -> Result<FrozenDesktop, CaptureError>; fn window(&self, id: WindowId, keep_shadow: bool) -> Result<Image, CaptureError>; }
pub trait Clipboard { fn set_image(&self, img: &Image) -> Result<(), ClipboardError>; fn set_text(&self, s: &str) -> Result<(), ClipboardError>; fn get_image(&self) -> Result<Option<Image>, ClipboardError>; }
pub trait SecretStore { fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>; fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError>; fn delete(&self, key: &SecretKey) -> Result<(), SecretError>; }
pub trait Ocr { fn recognise(&self, img: &Image, lang: Option<&str>) -> Result<OcrText, OcrError>; }
pub trait ImageCodec { fn encode(&self, img: &Image, fmt: ImageFormat, quality: u8) -> Result<Vec<u8>, CodecError>; fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError>; }
pub trait Notifier { fn toast(&self, n: Notice); }
pub trait Sound { fn play(&self, s: SoundId); }
pub trait Clock { fn now_local(&self) -> OffsetDateTime; }
pub trait OverlayHost { fn select_region(&self, req: OverlayRequest) -> Result<OverlayResult, OverlayError>; }
pub trait Recorder { fn start(&self, req: RecordRequest) -> Result<(), RecordError>; fn stop(&self) -> Result<(), RecordError>; }
pub struct Ports<'a> { pub capture: &'a dyn ScreenCapture, pub clipboard: &'a dyn Clipboard, pub secrets: &'a dyn SecretStore, pub ocr: &'a dyn Ocr, pub codec: &'a dyn ImageCodec, pub notify: &'a dyn Notifier, pub sound: &'a dyn Sound, pub clock: &'a dyn Clock, pub overlay: &'a dyn OverlayHost, pub recorder: &'a dyn Recorder }
```

`Secret` wraps its bytes in `zeroize::Zeroizing`, zeroes them on drop, and its `Debug` prints `***`.

**`snapline-store`**: settings file load, save, migrate (section 5.8); `HistoryRepo` with hand-written SQL (`page(filter, cursor)`, `get(id)`, `record(..)`, `set_upload(..)`, `delete(id)`, `stats()`); migrations as numbered `.sql` files in `crates/store/migrations/` embedded with `include_str!` and tracked with `PRAGMA user_version`.

**`snapline-upload`**: one `Uploader` trait, the upload queue, HTTP client, EXIF stripping, Imgur, S3 (SigV4 in-house), custom `.sxcu` uploaders, Gist, SFTP, link formats.

```rust
pub trait Uploader: Send + Sync { fn upload(&self, file: &UploadFile, secrets: &dyn SecretStore) -> Result<Uploaded, UploadError>; fn delete(&self, link: &DeletionLink, secrets: &dyn SecretStore) -> Result<(), UploadError>; }
pub enum UploadError { Retryable(String), TooLarge, Auth, Permanent(String) }
```

**`snapline-services`**: the application layer every frontend calls. It owns the worker and uploader threads, the `Job` and `AppEvent` enums, the action router, and one module per area with plain Rust functions (for example `history::page(&self, HistoryFilter, Option<Cursor>) -> Result<HistoryPage, ServiceError>`). It also owns the DTO types that cross IPC, which derive `serde::Serialize`/`Deserialize` and `ts_rs::TS`.

**`apps/snapline`**: `main.rs` builds Tauri, wires ports, starts threads; `tray.rs`; `windows.rs` (`WindowManager`); `ipc/<area>.rs` (one Tauri command per area, a thin wrapper over services); `protocol.rs` (the `thumb` URI scheme); `events.rs` (forwards `AppEvent` to webviews); `crash.rs` (panic hook); `logging.rs`; `cli.rs` (command-line arguments, including `--uninstall-cleanup`).

### 5.5 IPC contract

There is exactly **one Tauri command per area**, registered once in `main.rs` by task 1.0 and never edited again:

`ipc_app`, `ipc_settings`, `ipc_hotkeys`, `ipc_history`, `ipc_upload`, `ipc_workflow`, `ipc_record`, `ipc_tool`.

Each takes that area's request enum and returns that area's response enum or an `IpcError`:

```rust
#[tauri::command]
async fn ipc_history(services: tauri::State<'_, Services>, req: HistoryReq) -> Result<HistoryResp, IpcError> {
    let s = services.inner().clone();
    tauri::async_runtime::spawn_blocking(move || s.history().handle(req)).await.map_err(IpcError::join)?
}
```

- Request and response enums are `#[serde(tag = "kind", content = "data", rename_all = "camelCase")]`, defined in `crates/services/src/dto/<area>.rs` (one file per area, owned by the area's tasks). Adding a variant touches only that file and the area's handler.
- `IpcError` is `{ code: string, message: string }`. `code` is one of `not_found`, `invalid`, `busy`, `auth`, `io`, `network`, `internal`. `message` is UK English, user-readable, and never contains a secret or a full file content.
- Commands never block the main thread: all work runs in `spawn_blocking`, and anything that writes history goes to the worker as a `Job` with a reply channel.
- Events: one event name, `snapline://event`, payload `AppEvent`, `#[serde(tag = "area", content = "event")]` with one variant per area wrapping that area's event enum (`AppEvent::History(HistoryEvent)`, and so on). The top-level enum is created in task 1.0 with every area; area tasks only edit their own event enum.
- TypeScript types are generated from the Rust DTOs with `ts-rs` into `ui/src/lib/ipc/bindings/` by `cargo xtask bindings`. `cargo xtask check` fails if the generated files are out of date. Generated files are committed.
- Each area has a hand-written TypeScript wrapper `ui/src/lib/ipc/<area>.ts` with one typed async function per request (`historyPage(filter, cursor)`), built on a shared `call<Req, Resp>(cmd, req)` in `ui/src/lib/ipc/core.ts`.
- Images never cross IPC. Thumbnails are served by the custom scheme `thumb`: `http://thumb.localhost/<capture id>` on Windows. The handler parses the id as a positive integer (anything else is 404), looks up the thumbnail path through `HistoryRepo`, regenerates it if missing, and returns `image/webp` bytes. No other path can be reached.

### 5.6 Frontend

Stack: **Svelte 5** (runes), **TypeScript** (strict), **Vite**, no CSS framework, no UI library, no router library. Styling is plain CSS using the tokens as CSS custom properties. Svelte compiles to small vanilla JS with no virtual DOM, which keeps the renderer's heap small.

```
ui/src/
  main.ts                     reads location.hash, mounts App
  App.svelte                  picks the window shell by route prefix (#/panel or anything else)
  router.ts                   tiny hash router: route table, current route store, navigate()
  styles/
    tokens.css                GENERATED by `cargo xtask tokens` from design/tokens.toml
    base.css                  reset, fonts, focus ring, .hv hover, scrollbars, toggle
  lib/
    icons.ts                  every icon as an SVG path string, copied from design/v2
    format.ts                 sizes, times, durations, UK English
    events.ts                 subscribe to snapline://event, typed
    ipc/core.ts               call(), IpcError handling
    ipc/<area>.ts             one typed wrapper per area
    ipc/bindings/             GENERATED by ts-rs
  components/                 shared components (section 7.4), append-only after task 1.3
  windows/
    panel/QuickPanel.svelte
    main/MainShell.svelte, Nav.svelte, TitleBar.svelte
    main/history/...          HistoryPage, CaptureGrid, CaptureList, DetailPane
    main/workflows/...        WorkflowsPage, WorkflowEditor
    main/tools/...            ToolsPage, QrTool, CombinerTool, ConverterTool
    main/settings/...         SettingsShell, GeneralPage, CapturePage, HotkeysPage, UploadsPage, destination forms
```

Routes (all created in task 1.0 with placeholder pages; feature tasks fill the pages):

| Route | Window | Page |
|-------|--------|------|
| `#/panel` | panel | Quick panel |
| `#/history` | main | History |
| `#/workflows` | main | Workflows list |
| `#/workflows/new` and `#/workflows/<id>` | main | Workflow editor |
| `#/tools` | main | Tools grid |
| `#/tools/qr`, `#/tools/combine`, `#/tools/convert` | main | Tool pages that need a window |
| `#/settings/general`, `#/settings/capture`, `#/settings/hotkeys`, `#/settings/uploads` | main | Settings pages |

Rules:

- No network requests from the webview, ever. No Google Fonts (the design files load Inter only for the design canvas; ignore it). Fonts come from Windows.
- No `localStorage` or `IndexedDB` for anything that matters. The only browser storage allowed is none.
- Every list longer than 60 items is virtual (renders only visible rows plus one screen of overscan).
- Thumbnails use `<img loading="lazy" decoding="async">`, and rows that scroll out of the virtual window are removed from the DOM so their images are released.
- No animation loops. CSS transitions only, 150 ms or less, and `prefers-reduced-motion` turns them off.
- Svelte components follow the design's markup: real `<button>`, `<a>`, `<input>`, `<label>`, `<select>`, with the same `aria-*` attributes as the boards.

### 5.7 Security model

- The webview only ever loads the app's own bundled files. No remote URL, no `dangerousRemoteDomainIpcAccess`, no `withGlobalTauri` (the frontend imports `@tauri-apps/api` instead).
- CSP in `tauri.conf.json`: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' http://thumb.localhost data:; connect-src ipc: http://ipc.localhost; font-src 'self'; object-src 'none'; base-uri 'none'; frame-src 'none'`. `'unsafe-inline'` for styles is needed because Svelte sets inline `style` attributes; scripts stay strict.
- Capabilities (`apps/snapline/capabilities/main.json`) allow only: the eight `ipc_*` commands, `core:event:allow-listen`, `core:event:allow-unlisten`, and the window commands the custom title bar needs (`core:window:allow-minimize`, `allow-toggle-maximize`, `allow-close`, `allow-start-dragging`, `allow-is-maximized`). No fs, shell, dialog, http, clipboard or opener plugin permissions are exposed to JavaScript. File dialogs, clipboard, opening files and opening URLs are done in Rust, behind area requests.
- Secrets go in one direction only: the frontend can send a new secret value in a "save" request. Rust stores it in Credential Manager and from then on only reports `{ saved: true }`. No request ever returns a stored secret. A secret input shows `Saved` as a placeholder when a value exists and is empty otherwise.
- Opening a URL (for example "Open in browser" on a link) goes through `ipc_history` → Rust → `ShellExecuteW` with the URL, only for `https://` and `http://` URLs that came from History. Opening a file goes through Rust with a path taken from History by id, never a path sent by the webview.
- TLS: rustls with certificates from the Windows store (`rustls-platform-verifier`). No switch to turn checking off.
- EXIF is stripped before any upload. Uploading from the clipboard asks first (native dialog).
- `ffmpeg.exe` is started by absolute path, never through `PATH`, with arguments as a list, never a shell string.
- The `.sxcu` importer limits file size to 64 KiB and URL schemes to `https`, plus `http` with a warning. Export leaves header values blank unless the user ticks "Include header values".

### 5.8 Data

| What | Where | Format |
|------|-------|--------|
| Settings, workflows, destinations (no secrets) | `%APPDATA%\SnapLine\settings.toml` | TOML with a `version` field |
| History | `%LOCALAPPDATA%\SnapLine\history.db` | SQLite, WAL mode |
| Thumbnails | `%LOCALAPPDATA%\SnapLine\thumbs\<id>.webp` | 320 px wide WebP, regenerated if missing |
| Logs | `%LOCALAPPDATA%\SnapLine\logs\` | daily files, 7 kept |
| Crash reports | `%LOCALAPPDATA%\SnapLine\crashes\` | text, one per crash |
| WebView2 data | `%LOCALAPPDATA%\SnapLine\WebView2\` | managed by WebView2 |
| Captures | `%USERPROFILE%\Pictures\Snapline\<subfolder>\` (setting) | PNG, JPEG, WebP; MP4, WebM, MKV, GIF |
| Secrets | Credential Manager, target `SnapLine:<dest-id>:<field>` | `CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE` |
| Portable mode | a file named `portable` next to `snapline.exe` | settings, history, thumbs and logs go to `.\data\` next to the exe |

Settings writes go to `settings.toml.tmp` and are renamed over the original (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`). A `version` field drives migration functions at load. Unknown keys are kept. A file that fails to parse is copied to `settings.toml.bad-<unix seconds>` and defaults are used, with a toast "Settings could not be read, so defaults are in use. The old file was kept."

History schema (migration `0001_init.sql`):

```sql
CREATE TABLE capture (
  id           INTEGER PRIMARY KEY,
  kind         TEXT    NOT NULL CHECK (kind IN ('image','video','gif')),
  path         TEXT    NOT NULL,
  bytes        INTEGER NOT NULL,
  width        INTEGER,
  height       INTEGER,
  duration_ms  INTEGER,
  source_app   TEXT,
  created_at   INTEGER NOT NULL,
  ocr_text     TEXT
);
CREATE TABLE upload (
  id            INTEGER PRIMARY KEY,
  capture_id    INTEGER NOT NULL REFERENCES capture(id) ON DELETE CASCADE,
  dest_id       TEXT    NOT NULL,
  state         TEXT    NOT NULL CHECK (state IN ('queued','uploading','done','failed','cancelled')),
  url           TEXT,
  deletion_url  TEXT,
  error         TEXT,
  attempts      INTEGER NOT NULL DEFAULT 0,
  updated_at    INTEGER NOT NULL
);
CREATE INDEX capture_created ON capture(created_at DESC, id DESC);
CREATE INDEX upload_capture ON upload(capture_id, updated_at DESC);
CREATE VIRTUAL TABLE capture_fts USING fts5(name, source_app, ocr_text, content='');
```

`created_at` and `updated_at` are Unix milliseconds, UTC. `capture_fts` is kept in step by the repo (insert and delete in the same transaction as `capture`), not by triggers. Deletion links are not secrets (they can only delete that one upload), so they live here.

### 5.9 Dependencies

Use the latest stable release of each at the moment task 0.1 (Rust) or 0.3 (npm) runs, declare it once (workspace `Cargo.toml` or `ui/package.json`, exact version for npm), and never upgrade it mid-milestone. `cargo deny check` and `cargo xtask check` (npm licence check) must pass.

Rust:

| Crate | Used in | Why |
|-------|---------|-----|
| `tauri` (features: `tray-icon`, `image-ico`), `tauri-build` | apps/snapline | Windows, tray, IPC |
| `tauri-plugin-single-instance` | apps/snapline | Named mutex plus argument forwarding from a second launch |
| `windows` | platform-windows, overlay-windows | Microsoft's bindings for Win32, COM, WinRT. Only needed features, all listed in the workspace by task 0.1. |
| `webview2-com` (same version Tauri uses) | apps/snapline | `ICoreWebView2Settings3`, `ICoreWebView2_19` for the settings in 5.2 |
| `rusqlite` (feature `bundled`) | store | History database |
| `serde`, `serde_json`, `toml` | most | Settings, `.sxcu`, API responses, IPC |
| `ts-rs` | services | TypeScript types from Rust DTOs |
| `serde_json_path` | upload | JSONPath (RFC 9535) for custom uploader responses |
| `ureq` (features `rustls`, `platform-verifier`) | upload | Blocking HTTP, no async runtime of our own |
| `hmac`, `sha2` | upload | S3 SigV4 |
| `ssh2` | upload | SFTP (libssh2 on WinCNG, no OpenSSL). Decided here; `russh` is not used. |
| `img-parts` | upload | Strip EXIF from JPEG and PNG without re-encoding |
| `crossbeam-channel` | services, platform-windows | Thread messaging |
| `time` | core | Dates for naming tokens |
| `zeroize` | core | Secret buffers |
| `tracing`, `tracing-subscriber`, `tracing-appender` | all | Logs |
| `thiserror` (libraries), `anyhow` (apps, xtask) | all | Errors |
| `qrcode`, `rqrr` | core | Make and read QR codes |
| `rand` | core | `{rand}` naming token |
| `insta`, `proptest`, `tempfile` (dev only) | tests | Snapshots, property tests, temp files |
| `tiny_http` (dev only) | upload tests | Local stub HTTP server |

Refused: any async runtime of our own (Tauri's tokio is accepted because Tauri needs it, but our code never spawns tokio tasks other than `spawn_blocking`), `reqwest`, the `image` crate (WIC does this), `openssl`, any Tauri plugin not listed above (fs, shell, dialog, clipboard, http, notification, global-shortcut, autostart, opener), any UI component library.

npm (in `ui/package.json`, exact versions):

| Package | Kind | Why |
|---------|------|-----|
| `svelte`, `@sveltejs/vite-plugin-svelte`, `vite`, `typescript` | dev | Build |
| `@tauri-apps/api` | runtime | `invoke`, `listen`, window controls |
| `@tauri-apps/cli` | dev | `tauri build` for packaging, pinned to the Rust `tauri` minor version |
| `svelte-check` | dev | Type checking `.svelte` files |
| `prettier`, `prettier-plugin-svelte` | dev | Formatting |
| `vitest`, `jsdom`, `@testing-library/svelte` | dev | Component tests |
| `axe-core` | dev | Accessibility checks in tests (task 10.4) |
| `license-checker-rseidelsohn` | dev | npm licence check in `xtask check` |

---

## 6. Resource budgets

`cargo xtask budget` checks these. "Total" means the private working set of `snapline.exe` plus every process whose parent chain leads to it (WebView2 processes, `ffmpeg.exe`), summed.

| State | Total RAM (private working set) | Other | CPU |
|-------|------------------------------|-------|-----|
| Idle in tray, no window opened since start | under 15 MB | 0 WebView2 processes | 0%, no timers or polling |
| Idle in tray after windows were opened and closed, after linger + 10 s | under 15 MB, and within 3 MB of the first idle figure | 0 WebView2 processes | 0% |
| Quick panel open | under 90 MB | | 0% when nothing changes |
| Main window open on History with 60 thumbnails | under 120 MB | | 0% when nothing changes |
| Main window and quick panel open | under 150 MB | | |
| Region overlay open (no webview open) | under 40 MB plus one frozen copy of each monitor | | under 1 frame of input lag |
| Recording 1080p30 (no webview open) | under 150 MB with FFmpeg | | under 5% with a hardware encoder |

Timing budgets:

| Path | Target |
|------|--------|
| PrtSc to overlay visible | under 150 ms (no webview involved) |
| Tray click to quick panel visible, cold (no webview existed) | under 500 ms |
| Tray click to quick panel visible, warm (during linger) | under 200 ms |
| "Open Snapline" to main window visible with History rows, cold | under 800 ms |

How `budget` measures: start the release build with `--budget-run` (which skips launch-at-sign-in and toasts), wait 30 s, sample; open each window through the same code path a click uses (triggered by `--budget-open panel|main` messages sent to the running instance through the single-instance channel), sample; close; wait for linger plus 10 s; sample. Private working set per process comes from `GetProcessMemoryInfo` with `PROCESS_MEMORY_COUNTERS_EX2::PrivateWorkingSetSize` when supported, and otherwise by counting non-shared pages from `QueryWorkingSet`. Task 1.9 checks one sample against Task Manager by hand with Aaron. CPU comes from `GetProcessTimes` over a 10 s window. Children are found with `CreateToolhelp32Snapshot` by walking `th32ParentProcessID`.

`budget` opens windows on screen, so it only runs when Aaron says it is a good time (rule 6).

---

## 7. Design spec (v2)

The source of truth is the set of files in `design/v2/`. They are HTML "design component" files from the design canvas: the markup and inline styles show layout, sizes and colours, and the `<script type="text/x-dc">` block at the bottom of each holds the sample data, labels, icon paths and option lists. They will not render on their own (they need the canvas runtime `support.js`, which is not included). Read them as source.

| File | Screen |
|------|--------|
| `Main-v2.dc.html` | Tray quick panel (left-click) |
| `TrayMenu-v2.dc.html` | Tray context menu (right-click), with the Record submenu open |
| `App-v2.dc.html` | Main window. Its `view` prop switches between `history`, `workflows` and `tools`. The same file also contains `uploads`, `recordings` and `text` views: **those are not built** (v2 navigation has only History, Workflows and Tools). |
| `App-v2-Workflows.dc.html`, `App-v2-Tools.dc.html` | Wrappers that show `App-v2` on the Workflows and Tools views |
| `Settings-General-v2.dc.html` | Settings, General and default workflow |
| `Settings-Capture-v2.dc.html` | Settings, Capture |
| `Settings-Hotkeys-v2.dc.html` | Settings, Hotkeys |
| `Settings-Uploads-v2.dc.html` | Settings, Upload destinations |
| `Overlay.dc.html` | Capture overlay: frozen screen, selection and annotation toolbar |
| `canvas.json` | The canvas index (titles and sizes of the boards) |

The sample data in the boards (file names, apps, URLs, counts like "248 captures") is **placeholder**. Real screens show real data. Where this section and a board disagree, this section wins, because it records decisions made after the boards were drawn.

### 7.1 Tokens

`design/tokens.toml` is the only place colours are defined. `cargo xtask tokens` generates:

- `ui/src/styles/tokens.css`: `:root` holds the dark set, `@media (prefers-color-scheme: light) { :root { ... } }` holds the light set, plus type and radius variables.
- `crates/overlay-windows/src/tokens.rs`: `pub const DARK: Palette` and `pub const LIGHT: Palette` with every colour as `[f32; 4]` (premultiplied is not used; straight RGBA in 0.0 to 1.0).

Task 1.3 makes `design/tokens.toml` complete by adding the keys the boards use that the current file lacks. Final contents:

```toml
[dark]
bg = "#202020"
mica = "#1c1c1c"
layer = "#272727"
card = "#2d2d2d"
cardAlt = "#343434"
stroke = "rgba(255,255,255,0.08)"
strokeHi = "rgba(255,255,255,0.16)"
text = "#ffffff"
text2 = "#d0d0d0"
text3 = "#a8a8a8"
accent = "#60cdff"
onAccent = "#002a40"
accentSoft = "rgba(96,205,255,0.15)"
subtle = "rgba(255,255,255,0.06)"
ok = "#6ccb5f"
okSoft = "rgba(108,203,95,0.15)"
err = "#ff99a4"
errSoft = "rgba(255,153,164,0.14)"
shadow = "rgba(0,0,0,0.5)"
menuLayer = "#2c2c2c"

[light]
bg = "#f3f3f3"
mica = "#eeeeee"
layer = "#f9f9f9"
card = "#ffffff"
cardAlt = "#f5f5f5"
stroke = "rgba(0,0,0,0.09)"
strokeHi = "rgba(0,0,0,0.18)"
text = "#1a1a1a"
text2 = "#3b3b3b"
text3 = "#5f5f5f"
accent = "#005fb8"
onAccent = "#ffffff"
accentSoft = "rgba(0,95,184,0.10)"
subtle = "rgba(0,0,0,0.045)"
ok = "#0f7b0f"
okSoft = "rgba(15,123,15,0.10)"
err = "#c42b1c"
errSoft = "rgba(196,43,28,0.10)"
shadow = "rgba(0,0,0,0.18)"
menuLayer = "#f9f9f9"

[annotation]
red = "#e5484d"
orange = "#f76b15"
yellow = "#ffd60a"
green = "#30a46c"
blue = "#0090ff"
white = "#ffffff"
redHighlight = "rgba(229,72,77,0.32)"
orangeHighlight = "rgba(247,107,21,0.32)"
yellowHighlight = "rgba(255,214,10,0.55)"
greenHighlight = "rgba(48,164,108,0.32)"
blueHighlight = "rgba(0,144,255,0.28)"
whiteHighlight = "rgba(200,200,200,0.4)"
dim = "rgba(0,0,0,0.55)"
hintPill = "rgba(20,20,20,0.82)"
hintText = "#e6e6e6"

[type]
sans = "Segoe UI Variable Text"
sansFallback = "Segoe UI"
mono = "Cascadia Mono"
monoFallback = "Consolas"
baseSize = "13px"

[radius]
sm = "4px"
md = "6px"
lg = "8px"
```

CSS font stacks: `font-family: 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif` and for monospace `'Cascadia Mono', 'Cascadia Code', Consolas, ui-monospace, monospace`. The boards list `Inter` as a fallback; do not include it.

`wall`, `wall2` and `taskbar` in the boards are only the fake desktop behind the mock-ups. Do not build them.

### 7.2 Shared styles (from the boards' `<helmet>` CSS)

Put these in `ui/src/styles/base.css` exactly:

- `.hv:hover { box-shadow: inset 0 0 0 999px var(--subtle); }` and `.hv { cursor: pointer; }` for every hoverable control.
- Focus ring: `button:focus-visible, a:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid var(--text); outline-offset: 1px; }`.
- Toggle switch `.tg`: `appearance: none; width: 40px; height: 20px; border-radius: 10px; border: 1px solid var(--text3); background: transparent; position: relative; margin: 0;` with a 12 px knob at `top: 3px; left: 3px` in `var(--text2)`; when checked, background and border `var(--accent)`, knob at `left: 23px` in `var(--onAccent)`; knob moves with `transition: left .15s`.
- Body font size 13 px. Links inherit colour and have no underline.

### 7.3 Windows

Theme follows Windows: CSS `prefers-color-scheme` handles the webview; Rust reads `AppsUseLightTheme` at start and on `WM_SETTINGCHANGE` for the overlay and tray icon.

Mica: on Windows 11 (build 22000 or later) the main window is created `transparent(true)` with `Effect::Mica` from Tauri's window effects, and the root element's background is transparent so Mica shows through behind the navigation. On Windows 10 the root background is `var(--mica)`. Rust tells the page which case applies in the `ipc_app` → `Info` response (`hasMica: boolean`).

All windows use `decorations(false)` with a custom title bar, because the design puts a search box in the title bar. The title bar uses `data-tauri-drag-region`. Caption buttons (Minimise, Maximise or Restore, Close) are 46 px wide, full title bar height, icon-only with `aria-label`, and call Tauri window commands. Known cost: the Windows 11 snap layouts flyout does not appear on hover over the custom Maximise button. Win+Z and dragging to screen edges still work.

#### Quick panel (`Main-v2.dc.html`)

- Window: 344 px wide, height fitted to content (the page reports its height with `ipc_app` → `PanelResize { height }` after layout; Rust sets the window size). No taskbar button (`skip_taskbar(true)`), always on top, not resizable.
- Position: above the tray icon. Take the tray icon rectangle from the click event and the monitor's work area. Right edge 12 px in from the work area's right edge; bottom edge 12 px above the work area's bottom (the taskbar). If the taskbar is at the top, left or right, place the panel 12 px from that edge, next to the icon, fully inside the work area.
- Closes on focus loss, Esc, or any action inside it.
- Panel frame: `background: var(--layer); border: 1px solid var(--stroke); border-radius: 8px; box-shadow: 0 16px 40px var(--shadow)`. On Windows 11 also set the native rounded corners with `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)` on the window handle.
- Header (padding `14px 12px 12px 16px`, gap 12): 32 px app logo (accent square, radius 8, white corner-bracket mark, from the board's SVG), "Snapline" 14 px semibold, then two 36 px icon buttons: "Open Snapline" (opens main window on History) and "Settings" (opens main window on Settings General).
- Capture tiles: a 4-column grid, gap 6, padding `0 12px 12px`. Tiles are 56 px tall, radius 6, icon 20 px above a 12 px medium label. Tile 1 "Region" uses accent background and `onAccent` text; the others use `var(--card)` with a stroke border. Order and actions: Region (`capture_region`), Window (`capture_window`), Screen (`capture_screen`), Scrolling (`capture_scrolling`). The `title` tooltip is "Label (Chord)" using the live binding, for example "Region (PrtSc)".
- Recent: a section with top border, padding `10px 16px 12px`. A header row with "Recent" (12 px, `text3`, medium) and a "View all" link (12 px, accent) that opens the main window on History. Then a 3-column grid of the 3 newest captures: a 16:10 thumbnail (radius 4, stroke border) from `thumb://`, with a duration badge bottom-right for videos and GIFs (`rgba(0,0,0,0.72)` background, white 10 px text, for example "0:42"); under it an 11 px `text3` line with a green link icon when uploaded and the relative time ("2 min ago", "18 min ago", "1 h ago", "Yesterday", then "8 Nov"). Clicking a recent opens the main window on History with that capture selected. With no captures, show one line "No captures yet" in `text3` instead of the grid.
- Footer: `background: var(--mica)`, top border, padding `8px 8px 8px 16px`. "Last link" (12 px, `text3`), the last uploaded link in monospace 12 px `text2` with ellipsis (shown without `https://`), and a "Copy" button (32 px tall, card background, stroke border, copy icon). If there is no link yet, the footer shows "Last link" and "None yet" and the Copy button is disabled.

#### Tray icon and menu (`TrayMenu-v2.dc.html`)

- Icon: the app logo at 16 px (with a 32 px variant for high DPI) in `assets/icons/`. While recording, a red dot variant. While hotkeys are paused, a dimmed variant.
- Tooltip: "Snapline", "Snapline · hotkeys paused", or "Snapline · recording 0:42".
- Left-click opens the quick panel. Right-click opens the menu. Double-click opens the main window.
- The menu is Tauri's native menu (muda), which is a real Win32 menu. Items, in order (chord text is the live binding, blank if unbound or blocked):
  1. Capture region · PrtSc
  2. Capture window · Alt+PrtSc
  3. Capture full screen · Ctrl+PrtSc
  4. Scrolling capture · Ctrl+Alt+S
  5. Record screen ▸ submenu: Record video (MP4) · Shift+PrtSc; Record GIF · Ctrl+Shift+G; Record active window; separator; System audio (check item); Microphone (check item)
  6. separator
  7. Open Snapline
  8. Settings
  9. Pause hotkeys (becomes "Resume hotkeys" while paused)
  10. separator
  11. Exit
- While recording, item 5 is replaced by "Stop recording · <record chord>".
- Menu icons: 16 px versions of the board's icons, rendered to PNG at build time by `cargo xtask icons` from the SVG paths in the boards, stroke `text2` of the current theme (two sets, chosen at menu build time).
- Dark menus: muda must render dark menus when Windows is in dark mode. Task 1.2 checks this. If the muda version in use cannot, stop and ask Aaron (the option is to draw the menu natively in `platform-windows` with `TrackPopupMenuEx` and uxtheme's preferred-app-mode call behind a version check).

#### Main window (`App-v2.dc.html`)

- Window: default 1280 × 880 logical px, minimum 900 × 600, resizable, centred on the monitor with the cursor on first open; later opens restore the last size and position (stored in `settings.toml` under `[window.main]`, clamped to a visible monitor).
- Root: `background: var(--mica)` (transparent on Windows 11 with Mica), column layout, 13 px text.
- Title bar, 44 px tall, padding-left 16, gap 12: 16 px logo, "Snapline" (12 px), a centred search field (max width 420, height 32, card background, stroke border with `strokeHi` bottom border, radius 4, search icon, placeholder "Search by name, app, text in image…", hidden label "Search captures"), then the three caption buttons. Typing in search switches to History and filters it (debounced 200 ms). The title bar is the drag region except the search field and buttons.
- Body: navigation on the left, a content layer on the right.
- Navigation (`flex: 1 1 220px`, padding `4px 8px 16px`, gap 2):
  - "New capture" button: 40 px tall, accent background, `onAccent` text, radius 6, 13 px semibold, icon, and the region chord ("PrtSc") right-aligned at 11 px with 0.85 opacity. It runs `capture_region` (the main window stays open behind the overlay; the overlay is top-most).
  - Nav items, 38 px tall, radius 4: History (with the total capture count right-aligned in 12 px `text3`), Workflows, Tools. The active item has `var(--subtle)` background and a 3 × 16 px accent bar at left 4 px, vertically centred, and `aria-current="page"`.
  - A spacer.
  - Default destination chip: min height 44, `var(--subtle)` background, stroke border, radius 6, cloud icon and the default image destination's name (for example "Imgur", or "Save locally only"). It links to Settings › Upload destinations.
  - "Settings" link, 38 px, gear icon, opens Settings › General.
- Content layer: `background: var(--layer)`, 1 px stroke border on top and left, top-left radius 8.
- Page header (padding `20px 24px 32px` on the page, gap 20): `h1` at 28/36 px semibold, subtitle 12 px `text3`, and an optional accent action button (34 px).
  - History: title "History", subtitle "<n> captures · <size> on disk · <n> uploaded this week" (real numbers from `HistoryRepo::stats`). Filters on the right: a segmented radio group (`subtle` background, stroke border, radius 6, padding 2; options 30 px tall, 12 px; selected option has `var(--card)` background, `text`, weight 600 and `box-shadow: 0 1px 2px var(--shadow)`): All, Images, Videos, GIFs, Uploaded, Local only. Then Grid view and List view icon toggles (34 px, `aria-pressed`).
  - Workflows: title "Workflows", subtitle "A workflow is a hotkey plus the steps that run after the capture", action button "New workflow".
  - Tools: title "Tools", subtitle "Utilities that don't need a capture first", no action.
- History grid: grouped by day with `h2` "Today · 6 captures", "Yesterday · 4 captures", then dates "Thursday 6 November · 3 captures" (UK format, weekday day month, plus year when not the current year). Each group is a grid `repeat(auto-fill, minmax(176px, 1fr))`, gap 12. Card: a `button` with `aria-pressed` for selection, `var(--card)` background, radius 6, padding `6px 6px 10px`, gap 8; selected card has `border: 1px solid var(--accent); box-shadow: 0 0 0 1px var(--accent)`, others a stroke border. Thumbnail 16:10, radius 4, with the type and duration badge ("MP4 · 0:42", "GIF · 0:08") for timed captures. Under it the file name (12 px medium, ellipsis) and a row with "14:31 · 412 KB" (11 px `text3`) and a status badge: "Link" (`ok` on `okSoft`), "Local" (`text2` on `subtle`), "Failed" (`err` on `errSoft`), plus "Uploading" (`accent` on `accentSoft`) while in progress. Badge: radius 10, padding `1px 6px`, small icon.
- History list view (not drawn in the design): a table inside a card (`var(--card)`, stroke border, radius 6), header row 12 px `text3`, rows 52 px tall separated by stroke lines, columns: a 40 × 26 thumbnail and file name, Type ("PNG image", "MP4 video", "GIF animation"), Size, Captured ("Today, 14:31"), Status badge. Clicking a row selects it. This mirrors the board's table style used in its (unbuilt) Uploads view.
- Empty History: centred in the page, "No captures yet" (14 px semibold) and "Press PrtSc to capture a region." (12 px `text3`).
- Detail pane (`aside`, `flex: 1 0 320px`, min 280, max 360, left stroke border, padding 20, gap 18): "Details" `h2`; preview 16:10, radius 6; file name 15 px semibold (wraps anywhere); "<kind> · <day> at <time>" 12 px `text3`. A row of 34 px icon buttons: Annotate (accent, opens the image in the native editor), Copy image, Show in folder, Pin to screen, Delete (`err` colour; asks with a native confirm "Delete <name>? The file goes to the Recycle Bin." with buttons "Delete" and "Cancel"). For videos and GIFs, Annotate and Pin are hidden and "Open" (play in the default app) is shown first.
  - Sharing section (top border, "Link" `h2` and the status badge with the long label: "Uploaded", "Not uploaded", "Upload failed", "Uploading"):
    - Uploaded: a read-only monospace input with the link, a Copy link button, a Show QR code button (shows a QR of the link in a small popover inside the pane, rendered by Rust as an SVG string through `ipc_tool`). Then "Deletion link saved" with a "Copy deletion link" text button when a deletion link exists, or "Delete from <host>" when the host supports API deletion (Imgur, S3 with a saved ETag).
    - Local: "Saved on this PC only. Upload it to get a shareable link." and an accent "Upload to <default destination>" button.
    - Failed: "Upload failed: <error message>" and an accent "Retry" plus "Upload elsewhere…" (a native popup menu listing the configured destinations).
  - Info section: "Info" `h2` and key-value rows (12 px, 7 px vertical padding, bottom stroke): Dimensions ("1280 × 720"), Duration (timed only), File size, Captured from (process name, or "Desktop"), Destination ("—" when none), Saved to (folder relative to the user profile, for example `Pictures\Snapline\2026-11`).
  - No selection: the pane shows "Select a capture to see its details." centred in `text3`.
- Keyboard: arrow keys move the selection in the grid; Enter opens the file; Delete asks to delete; Ctrl+C copies the image.

#### Workflows tab (`App-v2.dc.html`, `view = workflows`)

- One card per workflow: `var(--card)`, stroke border, radius 6, padding `16px 20px`. Left: the name (14 px semibold; `text3` when off) and the chord in a mono 11 px pill with `strokeHi` border; under them the steps as pills (12 px, padding `4px 10px`, radius 12, `subtle` background) joined by small chevron icons. Right: an "Edit" button (32 px) and an "On"/"Off" label with a `.tg` toggle (`aria-label="Enable <name>"`).
- The default workflow's name is shown with " (default)" appended, cannot be deleted or disabled (toggle disabled with a tooltip "The default workflow runs on PrtSc and from the tray"), and its chord pill shows the Capture region chord.
- When a workflow's chord is blocked by a clash, the chord pill has `err` text and border, and a line under the steps reads "Off because <chord> is also used by <other>." in 12 px `err`.

#### Workflow editor (not drawn in the design; built from existing components)

Route `#/workflows/new` or `#/workflows/<id>`. Page header: title "New workflow" or the workflow's name, subtitle "Steps run in order after the hotkey is pressed." Content, top to bottom, using the Settings row styles (section 7.4):

1. Field row "Name" (text input).
2. Hotkey row "Hotkey" using the HotkeyField component (section 7.4), with the live clash check.
3. Toggle row "Enabled".
4. "Steps" section using the StepList component from Settings General (drag handle, number, label, and a remove icon button instead of the checkbox), keyboard reorderable (Alt+Up and Alt+Down on the focused row), and an "Add step" select under it listing every step type. Steps that need a parameter show it inline at the right of the row: Upload (destination select: "Default destination" or a configured one), Copy link (link format select), Capture (Region, Window, Screen, Scrolling, Last region).
5. A footer row: accent "Save", "Cancel", and for existing workflows "Duplicate" and "Delete" (`err` text, asks first with a native confirm).

Validation: name required (1 to 60 characters); the first step must be a Capture or Record step; "Copy link" requires an earlier "Upload"; "Copy text" requires an earlier "Extract text"; "Convert to GIF" requires an earlier Record step. Errors show under the row in 12 px `err`.

#### Tools tab (`App-v2.dc.html`, `view = tools`)

Grid `repeat(auto-fill, minmax(220px, 1fr))`, gap 12. Card: a button, padding 16, `var(--card)`, stroke border, radius 6, min height 140; a 40 px icon tile (radius 8, `accentSoft` background, `accent` icon), the name 14 px semibold, the description 12/18 px `text3`, and the chord in mono 11 px `text2` ("—" when none). Cards in this order:

| Card | Description | Default chord | Opens |
|------|-------------|---------------|-------|
| Colour picker | Pick any pixel on screen and copy its hex, RGB or HSL value. | Ctrl+Shift+C | native picker |
| Text capture (OCR) | Select an area and copy the text in it. | Ctrl+Shift+T | native overlay |
| Screen ruler | Measure distances and sizes in pixels. | Ctrl+Shift+M | native ruler |
| QR code | Scan a QR code on screen, or make one from a link. | — | `#/tools/qr` |
| Image editor | Open any image to annotate, crop or blur. | — | file dialog, then native editor |
| Pin to screen | Keep an image floating above other windows. | — | file dialog or clipboard, then native pin |
| Image combiner | Join several captures side by side or stacked. | — | `#/tools/combine` |
| Video converter | Turn recordings into GIF, WebM or a smaller MP4. | — | `#/tools/convert` |

Before milestone 9 builds a tool, its card is shown disabled with "Coming in a later version" as its description.

#### Settings (`Settings-*-v2.dc.html`)

Settings is a section of the main window, not a separate window. When a settings route is active, the main window's title bar text reads "Snapline Settings" and its search field is hidden; the navigation is replaced by the settings navigation:

- "Back to Snapline" link (36 px, `text2`, back-arrow icon), returns to the last main route.
- "Find a setting" search field (34 px, card background). Typing filters the rows on the current page by title and description, and shows matches from other pages as links under a "On other pages" heading.
- Nav items (40 px): General, Capture, Hotkeys, Upload destinations, with the same active style as the main nav.
- Content: `var(--layer)`, stroke border top and left, top-left radius 8, padding `28px 40px 48px`, scrolls vertically, inner column max width 1000, gap 28. Page `h1` 28/36 semibold. Sections: `h2` 14 px semibold, optional description 12 px `text3`, then rows with gap 4.

Row types (section 7.4 has the components): Toggle row, Select row, Segmented row, Field row (text input plus optional button), Range row (slider plus value).

Changes save immediately (debounced 300 ms for text fields; on change for the rest) through `ipc_settings` → `Update`. There is no Save button except on forms that say so (custom uploader, destination forms).

**General** (`Settings-General-v2.dc.html`):

- Version card: logo, "Snapline" (16 px semibold), "Version <x.y.z>". The board's "You're up to date" and "Check for updates" button are **not built** until auto-update exists.
- Startup: Launch at sign-in (toggle, on) "Start Snapline in the tray when you sign in to Windows"; Start minimised to tray (toggle, on) "Don't open the main window on launch"; Notifications (toggle, on) "Show a toast when an upload finishes or a link is copied". The board's "Check for updates automatically" row is **not built**.
- Saving: Save captures to (field, mono, button "Browse…", default `%USERPROFILE%\Pictures\Snapline`) "Captures are kept on this PC even after they are uploaded"; Subfolders (select: "By month (2026-11)", "By day (2026-11-08)", "By app (chrome)", "No subfolders"; the dates and app in the labels are computed from today and are examples) "Organise saved files automatically"; Image format (segmented: PNG, JPEG, WebP; AVIF hidden) "PNG is lossless; the others are smaller"; Lossy quality (range 40 to 100, step 1, default 90, shown as "90%") "Used for JPEG, WebP and AVIF" (show "Used for JPEG and WebP" while AVIF is hidden).
- File naming: "Click a token to add it. Tokens are filled in at the moment of capture." A card with "Name pattern" (mono input, accent 2 px bottom border, default `{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}`), token chips (28 px pills, mono 12 px, tooltip in brackets): `{app}` (Process name), `{window}` (Window title), `{yyyy}` (Year), `{MM}` (Month), `{dd}` (Day), `{HH}` (Hour), `{mm}` (Minute), `{ss}` (Second), `{rand}` (Random characters), `{n}` (Counter), `{w}` (Width), `{h}` (Height). Clicking a chip inserts it at the caret. Then a "Preview" box (`subtle` background) showing the pattern filled with the sample values `app=chrome, window=Pull-request-42, yyyy=2026, MM=11, dd=08, HH=14, mm=31, ss=07, rand=k3f9, n=0042, w=1280, h=720` plus the extension of the chosen format. The preview is computed by Rust (`ipc_settings` → `PreviewName`) using the same code that names real files.
- Default workflow · PrtSc: "What happens after a capture. Each hotkey can run its own workflow — this is the one PrtSc and the tray use." A StepList with checkboxes, in this default order and state: Copy image to clipboard (on), Open in editor to annotate (off), Save to folder (on), Upload to default destination (on, note on the right shows the destination name, for example "Imgur"), Copy link to clipboard (on), Play a sound (on). Rows are reorderable. Then: Copied link format (select: Direct link, Markdown, HTML <img>, BBCode, Page link) "What gets pasted when you press Ctrl+V"; Other workflows (a row whose control is a button "<n> workflows" that opens the Workflows tab) "Per-hotkey workflows are edited in the app under Workflows".

**Capture** (`Settings-Capture-v2.dc.html`):

- Screenshots: Include mouse cursor (toggle, off) "Draw the pointer into screenshots"; Capture delay (segmented: Off, 1 s, 3 s, 5 s; default Off) "Wait before capturing, so you can open a menu first"; Shutter sound (toggle, on) "Play a sound when a capture is taken"; Keep window shadow and rounded corners (toggle, on) "Window captures keep the Windows 11 shadow on a transparent background".
- Region selection: Freeze the screen while selecting (toggle, on, no description); Magnifier (toggle, on) "Pixel-accurate zoom next to the cursor"; Magnifier zoom (segmented 2×, 4×, 8×; default 4×); Snap to windows and controls (toggle, on) "Hover to pick a whole window, panel or button, then click"; Show size and position (toggle, on) "Width, height and coordinates while dragging"; Full-screen crosshair (toggle, off) "Guide lines across every monitor".
- Monitors: "Choose what a full-screen capture (Ctrl+PrtSc) includes." A diagram of the real monitor layout (each monitor a box scaled to fit, number and resolution inside, "· primary" on the primary one; boxes that will be captured get `accentSoft` background and a 2 px accent border, others `subtle` with `strokeHi`), a caption ("All monitors are captured as one 4480 × 1440 image", "Monitor 1 (primary) is captured", "The monitor with the cursor is captured"), and Full-screen capture uses (select: All monitors, Monitor under cursor, Primary monitor; default Monitor under cursor).
- Screen recording: Video format (select: "MP4 · H.264", "MP4 · H.265 (HEVC)", "WebM · VP9", "MKV · H.264"; default MP4 · H.264) "H.264 plays everywhere; H.265 is smaller"; Frame rate (segmented 24, 30, 60; default 30) "Frames per second"; Quality (select: High (recommended), Balanced, Small file, Lossless); Record system audio (toggle, on) "Sound playing on this PC"; Record microphone (toggle, off); Microphone device (select: "Default microphone" then the real capture devices by friendly name); Show recording border (toggle, on) "Outline the recorded area (not included in the video)"; FFmpeg location (field, mono, "Browse…") "Used to encode video. Found automatically." Default value is the bundled `ffmpeg.exe` path. The user may browse to another `ffmpeg.exe`; the value must be an absolute path to an existing file named `ffmpeg.exe`. `PATH` is never searched.
- GIF: Frame rate (segmented 10, 15, 20, 30; default 15) "Lower frame rates make much smaller GIFs"; Maximum width (select: Original size, 1280 px wide, 800 px wide, 480 px wide; default 800 px wide); Optimise colour palette (toggle, on) "Smaller files, slightly slower to encode".
- Scrolling capture: Delay between scrolls (range 100 to 1000 ms, step 50, default 300, shown as "300 ms") "Raise this for pages that load content as you scroll"; Stop after this many scrolls (select 10, 20, 30, 50, Unlimited; default 30); Remove sticky headers (toggle, on) "Hide headers and footers that repeat on every scroll".

**Hotkeys** (`Settings-Hotkeys-v2.dc.html`): rows are Field rows with a mono read-only field showing the chord and a "Change" button (the HotkeyField component).

- Capture: Capture region (PrtSc) "Runs the default workflow"; Capture window (Alt+PrtSc); Capture full screen (Ctrl+PrtSc); Scrolling capture (Ctrl+Alt+S); Last region again (Ctrl+Shift+R).
- Recording: Record video (Shift+PrtSc); Record GIF (Ctrl+Shift+G).
- Tools: Text capture (OCR) (Ctrl+Shift+T); Colour picker (Ctrl+Shift+C); Screen ruler (Ctrl+Shift+M); Upload from clipboard (Ctrl+Shift+V).
- Behaviour: Take over the PrtSc key (toggle, on) "Stops Windows opening Snipping Tool when you press PrtSc"; Warn about clashes (toggle, on) "Tell me when another app already uses a hotkey".
- Page intro under the `h1`: "If two actions share a chord, or another app owns it, neither is bound and you are told."
- A row whose chord is blocked shows, under its title, "Off: also used by <other action or workflow>." or "Off: another app is using this shortcut." in 12 px `err`, and its field has an `err` border.

**Upload destinations** (`Settings-Uploads-v2.dc.html`):

- Default destinations, "Where each kind of capture goes when “Upload” runs.": Images; Videos and GIFs; Text and code; Other files; Link shortener. Each a select listing the configured destinations that accept that kind, plus "Save locally only" (and "None" for Link shortener). Options for services that are not built (Pastebin, is.gd) are not shown. Defaults on a fresh install: Images "Save locally only", Videos and GIFs "Save locally only", Text and code "Save locally only", Other files "Save locally only", Link shortener "None". The board's defaults (Imgur, S3) are sample data, because nothing is configured on a fresh install.
- Accounts and uploaders: header buttons "Import ShareX uploader (.sxcu)" (secondary) and "Add destination" (accent, opens a native popup menu: Imgur, Amazon S3, GitHub Gist, SFTP, Custom uploader). A card listing destinations, each row 64 px min: a 36 px icon tile, name (14 px) and detail (12 px `text3`, for example "Image host · anonymous uploads", "Bucket my-bucket · eu-west-2", "Text and code snippets", "Your own server over SSH", "Custom uploader · imported from .sxcu"), a status badge with a dot ("Configured" in `ok`, "Not set up" in `text2`), and an action button (min width 96): "Configure", "Set up" or "Edit". Imgur's action is "Configure" (Client ID only; the board's "Sign in" needs OAuth, which is not in scope).
- Destination forms open inline below the list, replacing the custom uploader card position, one at a time:
  - Custom uploader (`Custom uploader · <name>`, "Compatible with ShareX .sxcu files. Response values use JSON paths."): a grid `repeat(auto-fit, minmax(240px, 1fr))`, gap `14px 16px`, of labelled fields: Name; Request method (POST, PUT, PATCH); Request URL (mono, full width); Body (Multipart form data, Binary, Form URL-encoded, JSON); File form field (mono); Link from response (mono, for example `$.data.url`); Thumbnail from response; Deletion link from response; Error message from response. Then "Headers": rows of header name and value (value is a password input with a show/hide eye button, `aria-pressed`) and an "Add header" text button. A footer with a top border: accent "Test upload", "Save", "Export .sxcu", and a status pill on the right ("Not tested yet", "Testing…", "Test upload succeeded · link copied", or "Test failed: <message>").
  - Imgur: Client ID (secret field). S3: Endpoint (default AWS), Region, Bucket, Access key ID, Secret access key (secret), Path prefix, Public URL pattern (default `https://<bucket>.s3.<region>.amazonaws.com/<key>`), ACL public-read (toggle). Gist (M9): Token (secret), Public (toggle). SFTP (M9): Host, Port (22), Username, Password or Private key file (choice), Remote folder, Public URL pattern. Each form has Test upload, Save, and "Remove destination" (`err`, asks first).
- Upload behaviour: Remove location and camera data (toggle, on) "Strip EXIF metadata before anything leaves your PC"; Ask before uploading from the clipboard (toggle, on) "Avoid sending something you copied by mistake"; Retry failed uploads (select: Don't retry, Once, 3 times, 5 times; default 3 times); Upload timeout (segmented 30 s, 60 s, 120 s; default 60 s); Keep upload history (select: Forever, 1 year, 90 days, 30 days; default Forever) "Links and deletion links shown in History"; Proxy (select: Use system proxy, No proxy, Custom proxy…; default Use system proxy; Custom shows a mono field `http://host:port`).

#### Capture overlay (`Overlay.dc.html`), native

Drawn with Direct2D, DirectComposition and DirectWrite, never a webview. Matching the board:

- The frozen image of each monitor fills that monitor's overlay window. Everything outside the selection is dimmed with `annotation.dim` (`rgba(0,0,0,0.55)`).
- Before a selection: a crosshair cursor; if "Full-screen crosshair" is on, 1 px guide lines across all monitors; the magnifier next to the cursor (a 120 × 120 logical px square at the chosen zoom with a 1 px pixel grid at 8×, the hex colour of the centre pixel and the cursor's X and Y under it, flipped to stay on screen); window and control snapping shows the hovered rectangle with a 2 px accent border.
- Selection: 1 px accent border; eight 9 × 9 px white handles with a 1 px accent border at the corners and edge midpoints; above the selection's top-left (8 px gap, or inside if there is no room), a 24 px pill (`layer` background, stroke border, radius 4, mono 12 px) with the size in semibold ("660 × 500") and "X 280 · Y 160" in `text3`. Sizes are physical pixels.
- When the workflow running has an annotate step (or the overlay was opened for editing), after the mouse is released the toolbar appears; otherwise releasing the mouse finishes the capture immediately.
- Toolbar: centred horizontally on the monitor that holds the selection, 14 px from its top; `layer` background, stroke border, radius 8, padding 5, gap 2, shadow `0 12px 32px rgba(0,0,0,0.45)`. Contents, left to right:
  - Tools (34 × 34 px, radius 5; active tool has `accentSoft` background and `accent` icon): Select and move (V), Rectangle (R), Ellipse (E), Arrow (A), Pen (P), Highlighter (H), Text (T), Step number (N), Blur or pixelate (B), Crop (C), Pick colour from screen (I).
  - A 1 × 22 px separator with 6 px margins.
  - Colours (28 × 34 px buttons with an 18 px swatch; selected swatch has a 2 px ring in `layer` then 2 px in `text`): Red, Orange, Yellow, Green, Blue, White (hex values in `[annotation]`). Default Red, except the Highlighter, which defaults to Yellow.
  - Separator. Stroke sizes (28 × 34 px; dot sizes 4, 7, 11 px): Thin, Medium, Thick, which mean stroke widths of 2, 4 and 8 physical px at 100% scaling (scaled by the monitor's DPI). Default Medium.
  - Separator. Undo (Ctrl+Z), Redo (Ctrl+Y; `text3` when nothing to redo).
  - Separator. Copy to clipboard (Ctrl+C), Save (Ctrl+S), Pin to screen, then an accent "Upload" button (34 px tall, icon and label, `Enter`), then Cancel (Esc).
- The active tool's hint shows in a 26 px pill centred 66 px from the top (`layer`, stroke border, radius 13, 12 px `text2`): "Select: click an annotation to move or resize it", "Rectangle: drag to draw · Shift for a square", "Ellipse: drag to draw · Shift for a circle", "Arrow: drag from tail to tip · drag the middle to curve it", "Pen: draw freehand", "Highlighter: drag over text", "Text: click to type a label", "Step: click to place <n> next", "Blur: drag over anything private", "Crop: drag the selection edges", "Colour picker: click to copy a pixel’s hex value". (The board says the highlighter "snaps to text lines"; that needs OCR and is deferred, so the hint drops it.)
- A keyboard hint pill centred 64 px from the bottom (`annotation.hintPill` background, `annotation.hintText`, radius 16, 32 px tall, 12 px, gap 16, key names in bold white): "Enter upload", "Ctrl+C copy", "Drag edges resize", "Right-click reselect", "Esc cancel". While selecting (no toolbar yet) it shows "Drag select", "Tab windows or controls", "Right-click or Esc cancel".
- Shapes as drawn in the board: rectangles with the stroke width and 8 px corner radius; step numbers as 26 px circles filled with the colour, white bold 13 px number, shadow `0 1px 3px rgba(0,0,0,0.35)`; text labels bold 16 px in the colour on a white rounded (4 px) background with `2px 6px` padding (on a dark background when the colour is White); highlighter strokes in the colour's highlight value; blur as a pixelate of 12 px blocks (exact and reproducible, so it is safe; a Gaussian blur can sometimes be reversed).
- Icons for the toolbar come from the same SVG paths as the web UI, drawn with `ID2D1DeviceContext5::CreateSvgDocument` (available on Windows 10 1703 and later).

### 7.4 Shared web components

Built in task 1.3 into `ui/src/components/`, each a `.svelte` file with typed props. Feature tasks may add new components but never change existing ones.

| Component | What it is |
|-----------|-----------|
| `Icon` | Inline `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">` with a path from `icons.ts`; size prop |
| `Logo` | The app logo SVG from the boards at a given size |
| `Button` | Variants `accent` (accent bg, `onAccent`, semibold), `secondary` (`layer` bg, stroke border with `strokeHi` bottom), `text` (accent text, no border), `danger-text`; heights 28, 32, 34 |
| `IconButton` | Square 32 or 34 px, icon only, required `label` for `aria-label`, optional `pressed` |
| `TitleBar`, `CaptionButtons` | Section 7.3 |
| `NavItem` | 38 or 40 px nav entry with active bar and optional count |
| `Card` | `var(--card)`, stroke border, radius 6 |
| `ToggleRow` | Whole row is a `<label>`: icon, title (14 px), description (12 px `text3`), "On"/"Off" (13 px `text2`, 24 px wide, right-aligned), `.tg` checkbox. Min height 68, padding `12px 20px`, gap 16, card styling |
| `SelectRow` | Icon, title, description, `<select>` (34 px, min width 220, `layer` bg) labelled by the title |
| `SegmentedRow` and `Segmented` | Radio group as in section 7.3 History filters; options min width 52 |
| `FieldRow` | Text input (34 px, optional mono) and optional secondary button, max width 460 |
| `RangeRow` | Range input (200 px wide, accent colour) and value text (44 px, right-aligned) |
| `StepList` | Rows 52 px min: drag handle button (`aria-label="Reorder: <label>"`), number, checkbox or remove button, icon, label, optional note on the right. Reorder by pointer drag and by Alt+Up/Alt+Down |
| `HotkeyField` | Mono read-only field with the chord and a "Change" button. Change puts it in capture mode ("Press keys…"; Esc cancels; Backspace clears to unbound). A chord needs at least one of Ctrl, Alt, Shift or Win, unless the key is PrtSc, Pause, or F1 to F24. While capturing it asks Rust for the plan preview and shows clash text live. Win+key chords that Windows reserves are refused with "Windows uses this shortcut." |
| `Badge` | Status pill: `ok`, `local`, `failed`, `uploading` |
| `Thumb` | 16:10 image from `http://thumb.localhost/<id>` with lazy loading, radius 4, a neutral `subtle` placeholder until loaded, optional duration badge |
| `EmptyState` | Centred title and line |
| `Popover` | Small floating panel for the QR code in the detail pane; closes on Esc and outside click |

### 7.5 Default hotkeys and the clashes in the design

Defaults (stored in the action catalogue, section M1 task 1.0):

| Action | Default |
|--------|---------|
| Capture region (runs the default workflow) | PrtSc |
| Capture window | Alt+PrtSc |
| Capture full screen | Ctrl+PrtSc |
| Scrolling capture | Ctrl+Alt+S |
| Last region again | Ctrl+Shift+R |
| Record video | Shift+PrtSc |
| Record GIF | Ctrl+Shift+G |
| Text capture (OCR) | Ctrl+Shift+T |
| Colour picker | Ctrl+Shift+C |
| Screen ruler | Ctrl+Shift+M |
| Upload from clipboard | Ctrl+Shift+V |
| Workflow "Annotate and share" (on) | Ctrl+Shift+A |
| Workflow "Bug report clip" (on) | **Ctrl+Alt+R** (the design says Ctrl+Shift+R, which clashes with Last region again) |
| Workflow "Grab text" (off) | **Ctrl+Alt+T** (the design says Ctrl+Shift+T, which clashes with Text capture) |
| Workflow "Save quietly" (off) | Ctrl+Alt+PrtSc |

The two bold changes are made by this plan because, under fail closed, the design's defaults would switch off "Bug report clip" and "Last region again" on a fresh install, and "Grab text" and "Text capture" the moment Grab text is turned on. A test (task 7.6) checks that the full default set, with every workflow enabled, has no internal clash.

**Open question for Aaron (do not change without his answer):** global hotkeys take the chord away from every other app. Several design defaults are common app shortcuts: Ctrl+Shift+T (reopen closed tab in browsers), Ctrl+Shift+C (copy in Windows Terminal, inspect in browser devtools), Ctrl+Shift+V (paste as plain text), Ctrl+Shift+A, Ctrl+Shift+G, Ctrl+Shift+M. Until Aaron decides, build the defaults above.

---

## 8. Build, checks and tests

### 8.1 Toolchain

- Rust stable, pinned in `rust-toolchain.toml` with target `x86_64-pc-windows-msvc`, Visual Studio Build Tools linker.
- Static C runtime (`-C target-feature=+crt-static` in `.cargo/config.toml`), so no VC++ redistributable is needed.
- Node.js LTS and npm for `ui/`. `npm ci` only (never `npm install` in CI), lockfile committed.
- The binary has `#![windows_subsystem = "windows"]`. `build.rs` runs `tauri_build::build()` with a Windows app manifest setting per-monitor DPI awareness v2, Common Controls 6, UTF-8 active code page, `asInvoker`, and Windows 10/11 supported OS GUIDs.
- Release profile: `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `debug = "line-tables-only"`.
- Lints (workspace): `unsafe_op_in_unsafe_fn = "deny"`; clippy `unwrap_used`, `expect_used`, `dbg_macro` deny; `pedantic` warn; all warnings are errors in `check`. Tests may use `unwrap` and `expect` (`#[cfg(test)]` modules allow them).
- Formatting: `rustfmt` defaults with `max_width = 100`; Prettier for `ui/` with `printWidth: 100`, `singleQuote: true`, `semi: true`.
- Files stay under about 400 lines; split a bigger one.

**Gotcha:** `tauri::generate_context!()` embeds `ui/dist` at compile time, so any `cargo build`, `clippy` or `test` that includes `apps/snapline` fails if `ui/dist` does not exist. `cargo xtask` builds the UI first. When running cargo by hand, run `cargo xtask ui` first.

### 8.2 `cargo xtask` commands

All tasks are Rust code in `xtask/`, no shell scripts.

| Command | Does |
|---------|------|
| `ui` | `npm ci` (if `node_modules` is missing or the lockfile changed) and `npm run build` in `ui/` |
| `tokens` | Regenerates `ui/src/styles/tokens.css` and `crates/overlay-windows/src/tokens.rs` from `design/tokens.toml`. `--check` fails if they are out of date. |
| `bindings` | Runs the `ts-rs` export and writes `ui/src/lib/ipc/bindings/`. `--check` fails if out of date. |
| `icons` | Renders tray menu icons (PNG, 16 and 32 px, dark and light) from the SVG paths in `ui/src/lib/icons.ts` into `apps/snapline/icons/menu/`. `--check` fails if out of date. |
| `no-comments` | Scans `.rs`, `.ts`, `.svelte`, `.css`, `.html`, `.toml`, `.sql` files (except generated ones and `design/v2/`) and fails on any comment other than a `// SAFETY:` line directly above `unsafe`. It skips string and template literals. If it flags something that is not a comment, rewrite the code; there is no allowlist. |
| `check` | `ui`, `tokens --check`, `bindings --check`, `icons --check`, `no-comments`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `npx prettier --check .`, `npx svelte-check --fail-on-warnings`, npm licence check (allow MIT, ISC, BSD-2-Clause, BSD-3-Clause, Apache-2.0, 0BSD, CC0-1.0, Unlicense, MPL-2.0, GPL-3.0-or-later, LGPL-3.0-or-later) |
| `test` | `cargo nextest run --workspace` (excluding tests marked `#[ignore = "desktop"]`) and `npx vitest run` |
| `test-desktop` | Runs the `#[ignore = "desktop"]` tests (clipboard, hotkeys, Credential Manager, HKCU, capture). Only one agent at a time, and only when Aaron says (they can touch shared machine state). |
| `ci` | `check`, then `test`, then `budget`. Run before every milestone commit. |
| `budget` | Section 6. Opens windows on screen: only when Aaron says. |
| `fetch-ffmpeg` | Downloads the pinned FFmpeg build, checks SHA-256 against `xtask/ffmpeg.lock`, unpacks `ffmpeg.exe` and its licence into `third_party/ffmpeg/` |
| `package` | Release build, then the installer and portable zip (milestone 10) |

### 8.3 Tests by layer

| Layer | What | How |
|-------|------|-----|
| `core` | Hotkey plan, chord parsing, naming tokens, overlay and recording state machines, annotation commands, workflow runner, stitching, ruler, colour, combiner layout, FFmpeg arguments | Unit tests with fake ports. `proptest` for: no chord is ever bound twice; undo after any edits returns the start state; chord normalising is idempotent. |
| Annotation output | Every shape and style | Software `Canvas` renders to PNG, compared with `insta` binary snapshots |
| `store` | Settings round trip, migrations, unknown keys kept, crash-safe write, every past history schema opens | Temp files with `tempfile` |
| `upload` | Each uploader, `.sxcu` import and export, retry decisions, SigV4 | `tiny_http` stub server on 127.0.0.1 with recorded responses; AWS's published SigV4 test vectors; ShareX example round trips |
| `services` | Each area handler, the action router, job flow | Fake ports, temp data folders |
| `platform-windows` | Clipboard, Credential Manager, hotkeys, Run key, WIC, capture | `#[ignore = "desktop"]` tests, throwaway names prefixed `SnapLineTest:` |
| `ui` | Components and pages | `vitest` with `jsdom` and `@testing-library/svelte`, IPC mocked through a fake `call()`; `axe-core` checks per page (milestone 10) |
| Whole app | Startup, idle, opening each window, one capture-to-link run | `cargo xtask budget` and a scripted smoke run, only when Aaron says |

---

## 9. How the work is run

### 9.1 One agent, in order (the default)

The plan works with a single agent doing tasks one after another. Do the milestones in order, 0 to 10. Inside a milestone, do the tasks in the order of the milestone's "Order" line. Each task ends with `cargo xtask check` and the tests for the crates and UI files it changed passing. When every task of a milestone is done, run `cargo xtask ci` (ask Aaron first, because `budget` opens windows), then make the milestone's single commit.

Working on one branch per milestone is enough for a single agent: `git switch -c m3 main`, commit freely on `m3`, then `git switch main && git merge --squash m3 && git commit` with the milestone message. Delete `m3` afterwards.

### 9.2 Several agents in parallel (optional)

Only if the tool running the plan supports several agents at once.

1. The integrator creates a milestone branch, for example `m3`, from `main`.
2. Each task runs in its own git worktree on its own branch made from the milestone branch: `git worktree add ..\snapline-wt\3.2 -b m3/3.2 m3`. Each worktree builds into its own `target\`.
3. A task is finished when `cargo xtask check` passes and the tests for what it changed pass. The integrator merges it into `m3`.
4. When every task is merged, the integrator runs `cargo xtask ci` once and squashes `m3` into one commit on `main`.
5. A task from a later milestone may start early on the later milestone's branch made from the current one, then `git rebase --onto main m3 m4` once `m3` lands.

Builds compete for CPU, so about four agents at once is the practical limit on one PC.

### 9.3 Shared files

Every task lists the files it **owns** (only that task edits them) and its **shared** edits. Two tasks that own the same file never run at the same time.

| File | Rule |
|------|------|
| `Cargo.toml` (workspace dependencies, `windows` features) | Task 0.1 adds every dependency and feature the whole plan needs. After that, only the integrator edits it. |
| `ui/package.json`, `ui/package-lock.json` | Task 0.3 adds every npm package. After that, only the integrator. |
| `Cargo.lock` | Never merged by hand. Keep the target branch's copy and rebuild. |
| `crates/*/src/lib.rs` module lists | One line per task. When two branches add a line at the same place, keep both. |
| `crates/core/src/actions.rs` (action catalogue) | Task 1.0 lists every action of the whole plan. Feature tasks never edit the list. |
| `crates/services/src/event.rs`, `job.rs`, `dto/mod.rs`, `actions.rs` (router) | Task 1.0 creates every area variant and routes every action prefix to a stub. Feature tasks edit only their area's files. |
| `apps/snapline/src/main.rs` and `ipc/mod.rs` | Task 1.0 registers all eight commands. After that, only the integrator. |
| `apps/snapline/tauri.conf.json`, `capabilities/main.json` | Task 1.1 only, then the integrator. |
| `ui/src/router.ts` | Task 1.0 adds every route of the whole plan with a placeholder page. Feature tasks replace their placeholder page file, never the route table. |
| `ui/src/components/` | Task 1.3 creates the set. Later tasks add new files only. |
| History migrations (`crates/store/migrations/NNNN_*.sql`) | Each schema change is a new numbered file. The integrator hands out numbers. |
| Settings `version` | A new section with defaults needs no bump. Changing existing keys does, and the integrator hands out the number. |
| `crates/overlay-windows/src/window.rs` | One task at a time; the integrator orders 3.3, 3.4, 5.2, 5.3, 5.4, 5.5, 8.5, 9.2, 9.3, 9.4, 9.6. |
| `xtask/src/main.rs` | One match arm per task. |
| `xtask/src/budget.rs` | 1.9, then 8.7, then 10.3. Never at the same time. |

### 9.4 Things only one agent may do at a time

- `cargo xtask test-desktop` (shared machine state: clipboard, global hotkeys, `HKCU` keys, Credential Manager).
- `cargo xtask budget` and any run of the app (single instance; it shows on Aaron's screen).

---

## 10. Milestones

Each milestone ends with a build that runs, and builds only on the ones before it. Status is tracked in the table at the end of this file.

### Milestone 0: Foundations

**Goal:** an empty but complete workspace where `cargo xtask ci` passes, and a tray-only Tauri binary that starts with no window.

**What it needs:**

- `git init` with branch `main`; `Cargo.toml` workspace with members `crates/core`, `crates/store`, `crates/upload`, `crates/platform-windows`, `crates/overlay-windows`, `crates/services`, `apps/snapline`, `xtask`; `rust-toolchain.toml`, `.cargo/config.toml`, `deny.toml`, `.gitignore`, `LICENSE` (GPLv3 full text), `README.md` (one paragraph: what SnapLine is, GPLv3, how to build with `cargo xtask ui` then `cargo build`).
- Every crate with its `lib.rs` contract as a `#![doc = "..."]` attribute saying what the crate owns and what it must not do (section 5.4), and `#![forbid(unsafe_code)]` where section 5.3 says.
- `ui/` scaffolded: Vite + Svelte 5 + TypeScript strict, `index.html` with `<div id="app">`, `main.ts`, an `App.svelte` that shows nothing yet, Prettier and svelte-check configured, `vitest` configured with `jsdom`.
- `apps/snapline`: a Tauri app with `app.windows: []`, a tray icon with one menu item "Exit", single-instance plugin, `RunEvent::ExitRequested` prevented unless Exit was chosen. `frontendDist` is `../../ui/dist`.
- `xtask` with `ui`, `check`, `test`, `ci`, `no-comments`, and a stub `budget` that only measures tray idle (full version in 1.9).
- ADRs in `docs/adr/` (context, decision, consequences; about one page each): 0001 Rust; 0002 Tauri with on-demand webviews and native overlay (section 3 and 5.2); 0003 fail-closed hotkeys; 0004 WebView2 settings (filled in by task 1.1); 0006 Svelte frontend and the IPC contract (section 5.5). (0005 is reserved for fallback B.)
- `design/tokens.toml` (section 7.1 contents) and `design/v2/` (the design files, already in the repo).

**Done when:** `cargo xtask check` and `cargo xtask test` pass; the release binary starts, shows a tray icon, has 0 WebView2 processes, and idles under 15 MB and 0% CPU (checked once with Aaron's permission).

**Tasks** (order: 0.1, then 0.2, 0.3, 0.4 together, then 0.5):

**0.1 Workspace and crates.**
- Create every file in "What it needs" except `ui/`, `xtask` commands beyond a skeleton, and the Tauri app.
- Workspace `Cargo.toml` declares **every** Rust dependency in section 5.9 with versions, and the `windows` crate features the whole plan uses: `Win32_Foundation`, `Win32_Graphics_Gdi`, `Win32_Graphics_Dwm`, `Win32_Graphics_Direct2D`, `Win32_Graphics_Direct2D_Common`, `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_DirectComposition`, `Win32_Graphics_DirectWrite`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Imaging`, `Win32_Media_Audio`, `Win32_Media_KernelStreaming`, `Win32_Media_Multimedia`, `Win32_Security`, `Win32_Security_Credentials`, `Win32_Storage_FileSystem`, `Win32_System_Com`, `Win32_System_Com_StructuredStorage`, `Win32_System_DataExchange`, `Win32_System_Diagnostics_ToolHelp`, `Win32_System_LibraryLoader`, `Win32_System_Memory`, `Win32_System_Ole`, `Win32_System_Power`, `Win32_System_ProcessStatus`, `Win32_System_Registry`, `Win32_System_SystemInformation`, `Win32_System_Threading`, `Win32_System_Variant`, `Win32_System_WinRT`, `Win32_System_WinRT_Direct3D11`, `Win32_System_WinRT_Graphics_Capture`, `Win32_UI_Accessibility`, `Win32_UI_Controls`, `Win32_UI_HiDpi`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Shell`, `Win32_UI_Shell_Common`, `Win32_UI_WindowsAndMessaging`, `Foundation`, `Foundation_Collections`, `Graphics`, `Graphics_Capture`, `Graphics_DirectX`, `Graphics_DirectX_Direct3D11`, `Graphics_Imaging`, `Media_Ocr`, `Globalization`, `Storage_Streams`, `Data_Xml_Dom`, `UI_Notifications`. If a feature name does not exist in the chosen `windows` version, stop and ask.
- Owns: everything it creates.
- Built: workspace, shared dependency versions, locked toolchain, static CRT, lints, profiles, licence, README, deny policy, all six library crates and the eight workspace members.

**0.2 xtask.**
- `ui`, `check`, `test`, `ci`, `no-comments` and the stub `budget` as in section 8.2. `no-comments` has unit tests: it flags `// x`, `/* x */`, `<!-- x -->`, `# x` in TOML, `-- x` in SQL; it ignores `"// not a comment"` and template literals; it allows `// SAFETY:` directly above a line containing `unsafe`.
- Owns: `xtask/`.
- Built: `ui`, `check`, `test`, `ci`, `no-comments` and a tray-idle `budget` stub, with comment-scanner unit tests. Check covers the frontend build, formatting, clippy, cargo-deny, Prettier, svelte-check and npm licences.

**0.3 Frontend scaffold.**
- `ui/` as in "What it needs", with every npm package from section 5.9 at exact versions and `package-lock.json`. Scripts: `build` (`vite build`), `check` (`svelte-check --fail-on-warnings`), `test` (`vitest run`), `format` (`prettier --write .`).
- `vite.config.ts`: `base: './'`, `build.target: 'es2022'`, `build.outDir: 'dist'`, `build.sourcemap: false`, `clearScreen: false`, server port 5173 strict.
- Owns: `ui/`.
- Built: Vite 8, Svelte 5, strict TypeScript 6.0.3, all npm packages from section 5.9 at exact versions, lockfile, Prettier, svelte-check and jsdom/vitest configuration. TypeScript 7 was rejected because svelte-check 4.7.8 does not support it yet.

**0.4 ADRs and design files.**
- ADRs 0001, 0002, 0003, 0006 as above, and 0004 as a stub with the heading and "Filled in by task 1.1".
- Owns: `docs/adr/`, `design/tokens.toml`.
- Built: ADRs 0001, 0002, 0003, 0006 and the 0004 stub; `design/tokens.toml` with the final section 7.1 contents.

**0.5 Tauri skeleton.**
- `apps/snapline` as in "What it needs". `tauri.conf.json`: `productName` "Snapline", `identifier` "io.github.aaronburt.snapline", `app.windows` empty, `app.withGlobalTauri` false, `app.security.csp` from section 5.7, `bundle.active` false for now.
- `main.rs` sets up logging (stub), the single-instance plugin (a second launch logs its arguments for now), the tray icon with "Exit", and the run loop with exit prevention.
- Owns: `apps/snapline/`.
- Built: Tauri 2 skeleton with no windows, single-instance logging, "Exit" tray menu, exit prevention, static DPI/manifest settings, CSP and application icons.

### Milestone 1: Basic UI shell

**Goal:** every v2 screen opens on demand with placeholder data and matches the design; webviews are created on open and destroyed on close; idle memory returns to the idle figure.

**What it needs:**

- The full skeleton of the IPC contract, event enum, action catalogue, router and routes (task 1.0), so later tasks never touch shared files.
- The window manager and webview lifecycle in section 5.2, and the gate measurement.
- Tray icon and the full native menu from section 7.3 with stub actions (each logs and, for unbuilt features, shows a toast "Coming in a later version").
- Tokens generated to CSS and Rust; `base.css`; the component set in section 7.4.
- Quick panel, main window (History, Workflows, Tools with placeholder data shaped like the boards), and the four Settings pages with placeholder values.
- Dark and light themes following Windows; Mica on Windows 11; custom title bar.
- Logging: `tracing` to `%LOCALAPPDATA%\SnapLine\logs\` daily, 7 kept, level `info` (settings can raise it to `debug` later without restart). A panic hook writes `crashes\<unix seconds>.txt` (message, location, backtrace, version, Windows build) synchronously and shows a native message box "Snapline stopped because of a problem. A report was saved to <path>." then exits.
- The full `budget` command (section 6).

**Done when:** every screen in section 7.3 opens from its entry point and matches the boards side by side (checked with Aaron); webviews are destroyed on close; the gate in section 5.2 passes; all budgets in section 6 for tray idle, quick panel and main window pass.

**Order:** 1.0 alone first. Then 1.1, 1.2, 1.3, 1.8 together. Then 1.4, 1.5, 1.6, 1.7 together. Then 1.9.

**1.0 Shared skeleton.** Runs alone.
- `crates/core/src/actions.rs`: `ActionId` (string newtype) and the catalogue: a `static` slice of `ActionDef { id, label, default_chord: Option<&str>, group, available_from_milestone }` for every action in the plan: `capture_region`, `capture_window`, `capture_screen`, `capture_scrolling`, `capture_last_region`, `record_video`, `record_gif`, `record_window`, `record_stop`, `tool_ocr`, `tool_colour`, `tool_ruler`, `tool_qr`, `tool_editor`, `tool_pin`, `tool_combine`, `tool_convert`, `upload_clipboard`, `app_open`, `app_settings`, `app_pause_hotkeys`, `app_exit`, and the pattern `workflow:<id>`. Default chords from section 7.5. A helper `available(id, current_milestone)`.
- `crates/services/src/`: `lib.rs` with `Services` (cheap to clone, holds `Arc`s), `job.rs` (`Job` enum: `Action(ActionId)`, `Area(AreaJob)` with one variant per area), `event.rs` (`AppEvent` with one variant per area, each wrapping `<area>::Event` defined in that area's module as an empty enum or with a placeholder variant), `actions.rs` (router: prefix `capture_` → capture module, `record_` → recording, `tool_` → tools, `upload_` → uploads, `workflow:` → workflows, `app_` → app; each target module has a `run(action, &Services)` that returns `NotYet`), `dto/mod.rs` with `pub mod` lines for `app`, `settings`, `hotkeys`, `history`, `upload`, `workflow`, `record`, `tool`, each with `Req` and `Resp` enums holding one placeholder variant `Ping` → `Pong`.
- `apps/snapline/src/ipc/`: `mod.rs` and one file per area with its `#[tauri::command]`; `main.rs` registers all eight in one `generate_handler!`.
- `ui/src/router.ts` with every route in section 5.6, each pointing to a placeholder page component file that shows the route name; `ui/src/lib/ipc/core.ts` and one wrapper file per area with a `ping()`; `ui/src/lib/events.ts`.
- `xtask bindings` command and the first generated bindings.
- Tests: the router sends every catalogue action to a module; every route resolves; `ping` round trips for each area (Rust unit test on the handlers, vitest with a mocked `invoke`).
- Owns: every file above.
- Built: the full action catalogue, the services skeleton and router stubs, all eight DTO areas with `Ping`/`Pong`, all eight IPC commands registered once, the complete hash route table with placeholder pages, typed wrappers, event subscription, `cargo xtask bindings` with staleness checking, and the router/route/ping tests. Generated bindings include uniquely named area event types.

**1.1 Webview lifecycle and the gate.** Needs 1.0.
- `apps/snapline/src/windows.rs` (`WindowManager`) and `webview_tuning.rs` as in section 5.2, including the ready handshake, linger timer, exit prevention, and the release-build WebView2 settings.
- Measure with Aaron's permission: tray idle, open main, close, after linger plus 10 s. Try the extra browser arguments one at a time and the memory target level; keep what helps; write the results and final settings into ADR 0004.
- If the gate fails, try fallback A. If that fails, stop and report to Aaron with the numbers.
- Owns: `apps/snapline/src/windows.rs`, `webview_tuning.rs`, `tauri.conf.json`, `capabilities/main.json`, `docs/adr/0004-webview2-settings.md`.
- Built: WindowManager for panel and main, hidden creation with theme background and shared WebView2 data directory, ready handshake, destroy-on-close with 15 s linger and generation-based cancellation, exit prevention, release WebView2 hardening, main-window memory target on minimise, capability file, and a temporary Open Snapline tray entry plus `--open-main` single-instance path for the gate. Shared DTO additions: `AppReq::Ready`, `AppResp::Ready`, `App::Navigate`.
- Gate measurement: PASS. Idle 1,605,632 bytes with 0 WebView2 processes; main window open 65,814,528 bytes across 6 processes; 25 s after close 3,600,384 bytes with 0 processes, 1,994,752 bytes above the first idle figure. `--disable-component-update` saved 3,342,336 bytes and is kept; `--disable-background-networking` was rejected; the optional memory target level saved 9,052,160 bytes, below the 10 MB threshold, and was removed. Results recorded in ADR 0004.

**1.2 Tray and menu.** Needs 1.0.
- `apps/snapline/src/tray.rs`: icon (normal, recording, paused variants), tooltip, left-click (panel), double-click (main), right-click menu exactly as section 7.3 with icons from `apps/snapline/icons/menu/` (task 1.3 generates them; until then, no icons), check items for System audio and Microphone bound to settings later (stub state now).
- Menu actions go through the services action router.
- Second launch (single-instance callback): opens the main window, or runs the action given by `--action <id>`.
- Check dark menus in dark mode (with Aaron's permission). If muda cannot, stop and ask (section 7.3).
- Owns: `apps/snapline/src/tray.rs`, `assets/icons/`.
- Built: full native menu in 7.3 order with `IconMenuItem` 16px theme-aware icons, chord labels from the action catalogue, Record screen submenu, System audio and Microphone check items, Open, Settings, Pause/Resume, Exit; left-click panel and double-click main; normal, recording and paused icons and the three tooltips; second launch opens main or runs `--action <id>`; tray state in a managed handle. Dark-menu rendering still needs a visual check with Aaron. Check items have no icon because Tauri's `CheckMenuItem` has no icon API.

**1.3 Tokens, base styles, components, icons.** Needs 1.0.
- Complete `design/tokens.toml` (section 7.1), `xtask tokens`, `ui/src/styles/base.css` (section 7.2), `ui/src/lib/icons.ts` (every icon path used in the boards, named by meaning: `region`, `window`, `screen`, `scroll`, `record`, `gif`, `ocr`, `colour`, `ruler`, `qr`, `edit`, `pin`, `copy`, `folder`, `upload`, `link`, `gear`, `open`, `search`, `cloud`, `server`, `code`, `trash`, `play`, `pause`, `power`, `check`, `chevronRight`, `back`, `minimise`, `maximise`, `restore`, `close`, `grid`, `list`, `drag`, `plus`, `eye`, `eyeOff`, `bell`, `speaker`, `mic`, `film`, `timer`, `cursor`, plus any other the boards use), `xtask icons`, every component in section 7.4 with a vitest test each (renders, labels, keyboard behaviour where it has any).
- Built: token generator with `--check`; generated CSS and overlay palette; 86 design icons plus caption icons in `icons.ts`; 92 tray menu PNGs (23 names × 16/32 × dark/light) from an in-house SVG rasteriser and PNG writer; base styles; all 20 components with a test each. `restore` and `eyeOff` were not drawn in any v2 board; Aaron approved the standard two-square Restore glyph and an eye-with-slash eyeOff, both added.
- Owns: `design/tokens.toml`, `xtask/src/tokens.rs`, `xtask/src/icons.rs`, `ui/src/styles/`, `ui/src/lib/icons.ts`, `ui/src/components/`, `crates/overlay-windows/src/tokens.rs`. Shared: one arm each in `xtask/src/main.rs`.

**1.4 Quick panel.** Needs 1.1 and 1.3.
- `ui/src/windows/panel/QuickPanel.svelte` exactly as section 7.3, with placeholder recents (three items shaped like the board) and a placeholder last link.
- Rust: placement above the tray icon, `PanelResize`, close on blur and Esc, `skip_taskbar`, always on top, rounded corners.
- Owns: `ui/src/windows/panel/`, `apps/snapline/src/panel.rs`, `crates/services/src/dto/app.rs` (panel requests), `crates/services/src/app.rs`.
- Built: QuickPanel exactly from the board with tiles, recents, last-link footer, tooltips and Esc/action close; panel placement maths for all four taskbar edges, 344 logical width, fitted height from `PanelResize`, focus-loss destroy and Win11 rounded corners. DTO adds `PanelResize`, `PanelClose`, `PanelAction`, `PanelCopyLink`, `OpenMain`, `OpenCapture`; bindings regenerated. Tests written, not run.

**1.5 Main window shell, History, Workflows and Tools with placeholder data.** Needs 1.1 and 1.3.
- `MainShell`, `TitleBar` use, `Nav`, page headers, History grid with the board's sample items, detail pane, filters (client-side over the sample data for now), grid and list toggle, Workflows cards with the five design workflows, Tools grid with the eight cards (disabled state as in 7.3).
- Built: MainShell with Mica-aware transparent root and settings routing; custom title bar with search, caption buttons and "Snapline Settings"; Nav with the fresh-install "Save locally only" destination; History grid/list, filters, detail pane and placeholder board data; five workflow cards; eight disabled tool cards. Tests written for filters, detail actions, default-workflow guard and tool cards.
- Owns: `ui/src/windows/main/MainShell.svelte`, `Nav.svelte`, `TitleBar.svelte`, `ui/src/windows/main/history/`, `ui/src/windows/main/workflows/WorkflowsPage.svelte`, `ui/src/windows/main/tools/ToolsPage.svelte`.

**1.6 Settings pages with placeholder values.** Needs 1.3.
- `SettingsShell` and the four pages exactly as section 7.3 with the default values listed there, held in a local placeholder object (wired to Rust in milestone 2).
- Built: SettingsShell with Back to Snapline, "Find a setting" filter and cross-page matches; General, Capture, Hotkeys and Uploads pages with every 7.3 row and default; local placeholder state; tests for token preview, defaults and filtering.
- Owns: `ui/src/windows/main/settings/`.

**1.7 Theme, Mica and title bar behaviour.** Needs 1.1.
- Windows 11 detection (`RtlGetVersion` build ≥ 22000), Mica effect, Windows 10 solid background, `hasMica` in `ipc_app` → `Info`, theme change while open (CSS follows automatically; Rust updates the window background colour and Mica dark mode flag with `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`).
- Owns: `apps/snapline/src/theme.rs`, `crates/platform-windows/src/theme.rs`. Shared: `crates/services/src/dto/app.rs` (`Info`, agreed with 1.4: 1.4 owns the file, 1.7 sends its variant to 1.4's owner or the integrator).
- Built: dynamic `RtlGetVersion` through ntdll with a registry fallback; Windows build/theme types in `snapline-win`; Mica effect on Win11 main window, solid `--mica` on Win10 and `--layer` panel; `AppReq::Info`/`AppResp::Info { hasMica }`; theme refresh and `DWMWA_USE_IMMERSIVE_DARK_MODE`; MainShell transparent root when `hasMica`. The services `app.rs` match arm was added for the new DTO variant.

**1.8 Logging and crash handling.** Needs 1.0.
- As in "What it needs". Test: a forced panic in a test binary writes a report file (the message box is skipped when `SNAPLINE_TEST=1`).
- Built: daily rolling logs in `%LOCALAPPDATA%\SnapLine\logs` with 7 kept and a non-blocking guard; bad-path console fallback; panic hook writing `crashes\<unix seconds>.txt` with message, location, backtrace, version and Windows build, native message box and exit, skipped under `SNAPLINE_TEST=1`. Wired before logging in `main`. Tests include a headless child-process panic test.
- Owns: `apps/snapline/src/logging.rs`, `apps/snapline/src/crash.rs`.

**1.9 Budget.** Needs 1.1, 1.4, 1.5.
- Full `cargo xtask budget` from section 6, including `--budget-run` and `--budget-open` handling in the app (through the single-instance argument channel).
- Check one sample against Task Manager with Aaron.
- Owns: `xtask/src/budget.rs`, `apps/snapline/src/cli.rs`.
- Built: typed CLI flags (`--budget-run`, `--budget-open`, `--action`, `--autostart`, `--uninstall-cleanup`); app opens budget windows through the normal WindowManager path; Rust budget runner samples private working set via GetProcessMemoryInfo with a QueryWorkingSet fallback, identifies real Snapline WebView2 descendants by creation time and data directory, samples CPU over 10 s, opens/closes panel and main, waits linger + 10 s, and enforces every M1 section 6 row. Tests written, not run. The Task Manager cross-check still needs Aaron.

### Milestone 2: Settings and hotkeys

**Goal:** settings are real and survive a restart; hotkeys work with the fail-closed rule; secrets go to Credential Manager.

**What it needs:**

- `settings.toml` load, save and migrate (section 5.8), one Rust module per section: `general`, `capture`, `hotkeys`, `uploads`, `workflows`, `window`. Every setting in section 7.3 with the defaults listed there.
- Settings pages bound to real values through `ipc_settings` (`Get`, `Update { patch }`, `PreviewName { pattern, format }`, `BrowseFolder { current }`), and `AppEvent::Settings(Changed)` so open pages refresh when something else changes a value.
- Hotkey registry with the fail-closed rule:
  - Collect chords of every enabled action and every enabled workflow. Disabled workflows don't count.
  - Normalise each chord (modifier order and left or right modifiers don't matter; `Ctrl+Shift+R` equals `Shift+Ctrl+R`; names: `Ctrl`, `Alt`, `Shift`, `Win`, keys as on the Hotkeys page: `PrtSc`, `A` to `Z`, `0` to `9`, `F1` to `F24`, `Pause`, `Insert`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `Up`, `Down`, `Left`, `Right`).
  - `core::hotkey_plan::plan(&[(Owner, Chord)]) -> HotkeyPlan { bind, blocked }`: a chord with exactly one owner is bound; a chord with more owners goes to `blocked` as `Internal { chord, owners }` and none of them is bound.
  - On the input thread: unregister everything, then `RegisterHotKey(NULL, id, mods | MOD_NOREPEAT, vk)` per bound chord. If Windows refuses (`ERROR_HOTKEY_ALREADY_REGISTERED`), that owner moves to `blocked` as `External`.
  - If anything is blocked and "Warn about clashes" is on, show one toast: "<n> hotkeys are off because of a clash" (or "1 hotkey is off because of a clash"), and mark the rows (section 7.3).
  - Re-run on every settings change that touches chords or enabled workflows, on resume from sleep (`PBT_APMRESUMEAUTOMATIC`), and when "Pause hotkeys" is turned off.
  - `ipc_hotkeys` → `Preview { owner, chord }` runs the same plan with the candidate chord against current bindings and the last known external failures, so the Hotkeys page shows clashes while typing.
  - Pause hotkeys: unregister all; the tray icon and tooltip change; Resume re-runs the plan.
- PrtSc takeover: when "Take over the PrtSc key" is on, set `HKCU\Control Panel\Keyboard\PrintScreenKeyForSnippingEnabled` (DWORD) to 0, remembering the previous value (or its absence) in `settings.toml` under `[hotkeys.restore]`; when switched off, on uninstall cleanup, restore it exactly. Windows reads the value at sign-in, so show under the toggle, the first time it is turned on: "Sign out and back in for Windows to release the PrtSc key." No low-level keyboard hook is used (a hook runs on every keystroke system-wide; this plan avoids it).
- Launch at sign-in: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value name `Snapline`, data `"<full exe path>" --autostart`. `--autostart` means: start in the tray without opening any window ("Start minimised to tray" off means the main window opens after autostart).
- Credential Manager wrapper: `CredWriteW`, `CredReadW`, `CredDeleteW`, `CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE`, target `SnapLine:<dest-id>:<field>`, user name `SnapLine`. Buffers zeroised.
- Toasts: WinRT `ToastNotificationManager` with AppUserModelID `io.github.aaronburt.snapline`. For unpackaged runs, register the AUMID under `HKCU\Software\Classes\AppUserModelId\io.github.aaronburt.snapline` with `DisplayName` "Snapline" and `IconUri` pointing to the installed icon PNG; the installer (milestone 10) also sets it on the Start menu shortcut. Clicking a toast opens the main window on the related capture. Toasts obey the Notifications setting.

**Done when:** settings survive a restart; two actions given one chord both show as off with the clash text and neither fires; a chord held by another app shows as off with the external text; secrets never appear in `settings.toml`, logs or IPC responses (test asserts this on a fixture).

**Order:** 2.1, 2.3, 2.6, 2.7, 2.8 together. Then 2.2, 2.4. Then 2.5.

**2.1 Settings model and file.** Owns: `crates/store/src/settings/` (`mod.rs`, `general.rs`, `capture.rs`, `hotkeys.rs`, `uploads.rs`, `workflows.rs` with an empty list for now, `window.rs`, `migrate.rs`), `crates/store/src/io.rs` (paths including portable mode, atomic write). Tests: round trip, defaults, unknown keys kept, bad file kept aside, migration from version 1 to itself (the framework).
- Built: portable/installed `Paths`; atomic settings write through `std::fs::rename` (MoveFileEx replace semantics) after `sync_all`; full version-1 Settings with every 7.3 default; unknown keys retained at every level; bad-file copy with the exact notice; migration framework with a 1→1 step; tests for defaults, round trip, unknown keys, bad file, portable paths and migration. `tempfile` added as a store dev-dependency.

**2.2 Settings service, IPC and page binding.** Needs 2.1. Owns: `crates/services/src/settings.rs`, `crates/services/src/dto/settings.rs`, `ui/src/lib/ipc/settings.ts`, and the binding code in `ui/src/windows/main/settings/` pages (replacing the placeholder object). Includes `PreviewName` (calls 4.2's naming code once it exists; until then returns the pattern with the extension) and `BrowseFolder` (native `IFileOpenDialog` with `FOS_PICKFOLDERS`, in `crates/platform-windows/src/dialogs.rs`, which this task owns).
- Built: `Services` now holds loaded settings behind a mutex with `with_paths`, snapshot/update and an optional folder-picker port; typed mirrored Settings DTO with ts-rs; Get/Update/PreviewName/BrowseFolder; deep JSON patch merge preserving unknown keys; Win32 folder dialog with cancel path; the four settings pages now load and save real values with 300 ms debounce on text fields. Bindings regenerated. Tests written, not run. `toml` was added to services (already a workspace dependency).

**2.3 Chords and the plan.** Owns: `crates/core/src/chord.rs`, `crates/core/src/hotkey_plan.rs`. Tests: parsing every key name, normalising, display strings exactly as section 7.5 writes them, `proptest` no chord bound twice.
- Built: `Modifiers`/`Key`/`Chord` with parsing, normalisation, display, reserved-key refusal, and the exact `Windows uses this shortcut.` message; `Owner`, `Blocked` with internal/external reasons, deterministic `HotkeyPlan`, `with_external`, and `preview`. Tests cover all listed key names, the 7.5 defaults, malformed input, internal/external clashes and two proptest properties. `proptest` added as a core dev-dependency. Aaron confirmed the common Windows-reserved Win+key set implemented.

**2.4 Input thread and hotkeys service.** Needs 2.1, 2.3. Owns: `crates/platform-windows/src/input.rs` (message-only window, `RegisterHotKey`, `WM_HOTKEY`, power and setting-change messages), `crates/platform-windows/src/prtsc.rs`, `crates/services/src/hotkeys.rs`, `crates/services/src/dto/hotkeys.rs`, `ui/src/lib/ipc/hotkeys.ts`. Desktop tests (`#[ignore = "desktop"]`): register and unregister a throwaway chord; detect an externally registered chord.
- Built: dedicated input thread with a hidden receiver window, `RegisterHotKey`/`MOD_NOREPEAT`, rebind/pause/resume/shutdown and external-clash detection; PrtSc takeover with exact restore; fail-closed hotkey service with internal/external blocks, exact clash notices, preview, pause/resume, resume-from-sleep and settings-change replans; typed Get/Preview/Pause/Resume DTOs and bindings. Input thread, notifier and PrtSc registry wired into main; tray Pause now drives the service. Tests written, not run. Receiver is a hidden top-level window because `HWND_MESSAGE` cannot receive `WM_SETTINGCHANGE`/`WM_POWERBROADCAST`.

**2.5 Hotkeys page live.** Needs 2.2, 2.4. Owns: `ui/src/windows/main/settings/HotkeysPage.svelte` and its test; uses `HotkeyField` from 1.3.
- Built: every 7.3/7.5 hotkey row bound to real settings and live `Get`, `Preview` calls while capturing, internal/external blocked text with err styling, chord updates, PrtSc sign-out hint on the first enable in a run (in-memory, no browser storage), and the exact intro/toggle text. Tests written, not run.

**2.6 Launch at sign-in.** Owns: `crates/platform-windows/src/autostart.rs`, plus `--autostart` handling in `apps/snapline/src/cli.rs` (shared with 1.9's file: add the flag only).
- Built: Run key value `Snapline` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` with `"<exe>" --autostart`, safe enable/disable/query wrappers, idempotent writes and a desktop round-trip test. `--autostart` starts in the tray unless "Start minimised to tray" is off, in which case main opens; toggling Launch at sign-in updates the Run key in the IPC settings handler.

**2.7 Credential Manager.** Owns: `crates/platform-windows/src/credentials.rs`, `crates/core/src/secret.rs`. Desktop tests with `SnapLineTest:` targets, cleaned up after.
- Built: zeroising `Secret` with `***` Debug and constant-time-style equality; validated `SecretKey` with target `SnapLine:<dest-id>:<field>` and user `SnapLine`; Win32 CredWrite/CredRead/CredDelete wrapper with `CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE`, idempotent delete, zeroed CredFree guard and no secret in errors. Desktop tests written with RAII cleanup, not run.

**2.8 Toasts and sounds.** Owns: `crates/platform-windows/src/notify.rs`, `crates/platform-windows/src/sound.rs` (`PlaySoundW` with `SND_ASYNC | SND_FILENAME` for `assets/sounds/shutter.wav` and `assets/sounds/done.wav`; the WAV files are Aaron's to provide; until then use `SND_ALIAS` with `SystemAsterisk`).
- Built: WinRT toasts with the exact AUMID and ToastGeneric XML, HKCU AppUserModelId registration/removal, activation callback with a capture route, and typed errors. Sounds use the two WAV paths with the `SystemAsterisk` alias fallback when the files are absent. Tests written, not run.

### Milestone 3: Screenshot

**Goal:** PrtSc shows the native overlay fast; region, window and screen captures land on the clipboard at the right size on mixed-DPI setups.

**What it needs:**

- Freeze: on capture start, copy every monitor with DXGI Desktop Duplication (`IDXGIOutputDuplication::AcquireNextFrame` once per output, copy to a staging texture). If duplication fails (for example on a remote desktop session), fall back to GDI `BitBlt` from the screen DC with `CAPTUREBLT`. Include the cursor only when "Include mouse cursor" is on (draw it from `GetCursorInfo` and `DrawIconEx`).
- Window capture: Windows.Graphics.Capture for the window under the cursor (`capture_window` uses the foreground window), which captures windows behind others. With "Keep window shadow and rounded corners" on, capture the window's extended frame bounds plus its shadow margin on a transparent background; off, crop to `DWMWA_EXTENDED_FRAME_BOUNDS`. On Windows 11 set `IsBorderRequired = false` (needs `GraphicsCaptureAccess.RequestAccessAsync(Borderless)`); on Windows 10 the yellow border may flash, which is accepted.
- Overlay: one borderless, top-most, tool window per monitor, per-monitor DPI aware v2, drawn with DirectComposition and a Direct2D swap chain; each frozen image uploaded to the GPU once; only the dim layer, selection, magnifier and annotations redraw, only when input changes them.
- The overlay's behaviour as a pure state machine in `core` (`Overlay::on(self, Input) -> (Overlay, Vec<Effect>)` with states `Hovering { snap }`, `Dragging { from }`, `Adjusting { rect, grip }`, `Annotating { rect, tool }` and effects `Redraw(Rect)`, `Finish(Selection)`, `Cancel`, `CopyColour(Rgb)`).
- Snapping: top-level windows listed once at overlay start (`EnumWindows`, visible, not cloaked, `DWMWA_EXTENDED_FRAME_BOUNDS`), and the control under the cursor through UI Automation (`IUIAutomation::ElementFromPoint`, bounding rectangle), queried on a separate thread with a 30 ms deadline; late answers are dropped. Tab switches between window and control snapping.
- Magnifier and size readout (section 7.3), Esc and right-click cancel, right-click with a selection resets to selecting.
- Capture delay setting (1, 3, 5 s): a small native countdown in the corner of the monitor with the cursor before freezing.
- Monitors setting for full-screen capture.
- "Last region again": store the last region (monitor id plus physical rect) in memory and in `settings.toml` `[capture.last_region]`; capture it without the overlay.
- Clipboard: PNG (registered format "PNG") and `CF_DIBV5` with alpha, set together; text as `CF_UNICODETEXT`. A message-only window owns the clipboard. Retry `OpenClipboard` up to 10 times, 10 ms apart, then fail with "The clipboard is busy. Try again."
- Shutter sound when on.

**Done when:** PrtSc shows the overlay in under 150 ms (measured from `WM_HOTKEY` to first present, logged at `info`); on a 100% plus 150% monitor setup the clipboard holds the selected region at its physical pixel size (checked with Aaron, by reading the clipboard's PNG dimensions in a desktop test, not by pasting into another app); window capture works for a window partly behind another.

**Order:** 3.1, 3.2, 3.5 together. Then 3.3. Then 3.4, 3.6.

**3.1 Overlay state machine and geometry.** Owns: `crates/core/src/geometry.rs`, `crates/core/src/overlay/` (`mod.rs`, `state.rs`, `snap.rs`). Tests for every transition, handles and edges, keyboard nudges (arrow keys move the selection 1 px, Shift+arrow 10 px, Alt+arrow resizes).
- Built: `Px`/`Point`/`Size`/`Rect`/`DpiScale`/`MonitorGeometry`; grips, handles, DPI conversions; `Overlay::on` with Hovering/Dragging/Adjusting/Annotating, all effects and inputs; window/control snapping and Tab cycling. 44 tests written including proptest properties, not run. Snap ordering and the active grip for Alt+arrow were the plan's ambiguities.

**3.2 Capture backends.** Owns: `crates/platform-windows/src/capture/` (`mod.rs`, `dxgi.rs`, `gdi.rs`, `wgc.rs`), `crates/platform-windows/src/monitors.rs`. Desktop tests: freeze returns one image per monitor at physical size.
- Built: monitor enumeration with stable ids/work areas/DPI; DXGI desktop duplication with D3D11 staging copies and cursor option; GDI BitBlt CAPTUREBLT fallback; WGC window capture with shadow/extended-bounds handling and Win11 borderless attempt; RAII for frames, maps, DCs, DIBs and WinRT apartment. Desktop tests written, not run. Open point for Aaron: `GraphicsCaptureAccess`/`RequestAccessAsync(Borderless)` needs the `Security_Authorization_AppCapabilityAccess` windows feature, which is not in the section 5.9/0.1 feature list; without it the Win11 borderless request is attempted through `SetIsBorderRequired(false)` only.

**3.3 Overlay windows and rendering.** Needs 3.1, 3.2, 1.3 (tokens). Owns: `crates/overlay-windows/src/` (`lib.rs`, `thread.rs`, `window.rs`, `d2d.rs`, `render.rs`, `magnifier.rs`, `readout.rs`, `icons.rs`). Implements `OverlayHost`.
- Built: native per-monitor topmost overlay windows with a DirectComposition/D3D11/D2D swap chain, frozen-image upload, core state-machine input, dim/selection/handles/snap rect, magnifier, size/readouts, exact toolbar and hint pills, SVG icons, edit mode and result channel. Toolbar/DPI/magnifier tests written, not run. No device-loss recovery yet; the overlay API is in the overlay crate pending core-neutral `OverlayHost` types in M3.6.

**3.4 Snapping.** Needs 3.3. Owns: `crates/platform-windows/src/windows_enum.rs`, `crates/platform-windows/src/uia.rs`. Shared: `crates/overlay-windows/src/window.rs` (snap target feed).
- Built: z-ordered visible/non-cloaked/non-tool window enumeration with extended frame bounds; UIA element query on a dedicated 30 ms-deadline thread with generation tokens and late replies dropped; overlay snap feed, Tab switching, hover clear. Tests written, not run. The overlay currently includes the two platform modules by `#[path]` to avoid a crate dependency; a later snap-source port is the cleaner architecture.

**3.5 Clipboard.** Owns: `crates/platform-windows/src/clipboard.rs`. Desktop tests: PNG and DIBV5 decode to the same pixels.
- Built: one OpenClipboard transaction publishing registered `PNG` and `CF_DIBV5` with alpha plus `CF_UNICODETEXT`, 10×10 ms retry with the exact busy message, PNG via WIC and DIBV5 fallback, HGLOBAL/session RAII, top-down BITMAPV5HEADER, and desktop tests that restore clipboard text. The core `Clipboard` port exists; an impl can be added without changing this module.

**3.6 Capture service.** Needs 3.2, 3.3, 3.5. Owns: `crates/services/src/capture.rs`, `crates/platform-windows/src/countdown.rs`. Wires `capture_region`, `capture_window`, `capture_screen`, `capture_last_region` through the router (fixed behaviour for now: capture then copy; workflows arrive in M7, saving in M4).
- Built: CaptureService with core capture/overlay/clipboard/countdown ports, physical-pixel region crop, all three full-screen monitor modes, window capture, last-region persistence, delay/countdown, router dispatch; native 88px countdown window. Tests written, not run. The app now installs the service with `CaptureBackend`, `OverlayHostImpl`, clipboard, countdown and a capture context. Known limits: frozen desktop is cloned for cropping; `[capture.last_region]` shape is implementation-defined; freeze always on; shutter sound waits for M4.

### Milestone 4: Saving to disk and History

**Goal:** every capture is saved with the right name in the right folder and shows in History and the quick panel, and survives a restart.

**What it needs:**

- WIC encoding (and decoding) for PNG, JPEG and WebP. JPEG and WebP use the Lossy quality setting. WebP uses the WIC WebP encoder when present; if Windows has no WebP encoder, the WebP option is shown disabled with "Needs the WebP Image Extension from the Microsoft Store". (Thumbnails are WebP too: if there is no WebP encoder, thumbnails are written as JPEG with the same name and `.jpg`, and the `thumb` scheme serves the right type.)
- File naming tokens (section 7.3): `{app}` is the process name without `.exe` of the window under the selection's centre (or "Desktop"), `{window}` is its title, characters invalid in Windows file names (`<>:"/\|?*` and control characters) replaced with `-`, trimmed to 80 characters; `{n}` is a counter stored in `settings.toml`, 4 digits, incremented per saved file; `{rand}` is 4 lower-case letters and digits. If the file exists, append ` (2)`, ` (3)` and so on.
- Subfolders setting: by month `yyyy-MM`, by day `yyyy-MM-dd`, by app `{app}`, or none.
- SQLite History (section 5.8) at `%LOCALAPPDATA%\SnapLine\history.db`, WAL mode, `foreign_keys = ON`. The worker owns the only write connection; IPC reads open their own read-only connections (one per `spawn_blocking` call is fine; reuse with a small pool of at most 2).
- `HistoryRepo`: `record(NewCapture) -> CaptureId`, `page(HistoryFilter, Option<Cursor>) -> HistoryPage` (60 rows, keyset pagination on `(created_at, id)`), `get(id)`, `stats() -> { count, bytes, uploaded_this_week }` (week starts Monday 00:00 local time), `delete(id)`, `search(text)` (FTS5, prefix match per word), `set_upload(..)` (used from M6).
- History page wired to real data: day groups, filters (All, Images, Videos, GIFs, Uploaded, Local only), search from the title bar, virtual grid and list (render only visible rows plus one screen of overscan; page in more as the user scrolls), detail pane, keyboard navigation, and the actions: Copy image, Show in folder (`SHOpenFolderAndSelectItems`), Open (default app), Delete (to the Recycle Bin with `IFileOperation` and `FOFX_RECYCLEONDELETE`, plus the row and thumbnail).
- Thumbnails: 320 px wide (height by aspect), written by the worker after saving, served by the `thumb` scheme, regenerated if missing.
- Quick panel recents and the History count in the nav from real data. `AppEvent::History(Added { id })` and `Removed { id }` refresh open pages.
- A fixed pipeline for now (the real workflow engine comes in M7): capture, copy image (if the default workflow's "Copy image" is on), save (if "Save to folder" is on), record in History, toast "Saved <file name>" when notifications are on.

**Done when:** every capture lands in the right folder with the right name and shows in History and the quick panel; restarting keeps it all; opening every past schema version works (tests); History with 10,000 synthetic rows scrolls without loading more than 3 pages at once (vitest with a fake IPC, plus a Rust test of the repo).

**Order:** 4.1, 4.2, 4.3 together. Then 4.4, 4.7. Then 4.5, 4.6.

**4.1 WIC codec.** Owns: `crates/platform-windows/src/wic.rs`. Desktop tests: round trip each format; WebP availability detection.
- Built: per-thread COM/WIC `WicCodec` with PNG/JPEG/WebP encode and decode through BGRA8, JPEG/WebP quality, WebP availability detection and 320px thumbnail scaling with WebP/JPEG fallback. Desktop round-trip tests written, not run. The codec is per-thread (`!Send`/`!Sync`), so the worker must own it.

**4.2 Naming.** Owns: `crates/core/src/naming.rs`. Tests for every token, invalid characters, collisions, the sample preview in section 7.3.
- Built: all naming tokens, Windows-invalid and control-character replacement, 80-character window title limit, 4-digit counter/wrap, deterministic random seam, collision suffixes, reserved device-name repair, sanitisation, and the exact sample preview. 19 tests written, not run. Choices reported: unknown braces stay literal, counter wraps at 9999 to 0000, reserved names get `_` appended.

**4.3 History store.** Owns: `crates/store/src/history/` (`mod.rs`, `repo.rs`, `migrate.rs`), `crates/store/migrations/0001_init.sql`. Tests against temp files.
- Built: exact 0001 schema, embedded numbered migrations with `PRAGMA user_version`, WAL/FK/busy timeout, read-write and read-only connections; `HistoryRepo` record/page/get/stats/delete/search/set_upload with 60-row keyset pagination, FTS5 prefix search and transactional FTS upkeep, Monday-local week stats and cascade delete. Tests written, not run. `busy_timeout = 5000 ms` is the only value the plan did not specify.

**4.4 Thumbnails and the `thumb` scheme.** Needs 4.1, 4.3. Owns: `crates/services/src/thumbs.rs`, `apps/snapline/src/protocol.rs`. Tests: non-numeric and negative ids return 404; missing thumbnails are regenerated.
- Built: strict positive-id parsing, existing WebP/JPEG served without rewrite, on-demand 320px regeneration from History, atomic writes, WebP/JPEG MIME and caching headers, 404 with no filesystem read, and scheme registration in main. Tests written, not run. **Architecture issue:** the implementation added `snapline-win` as a services dependency to reach WIC; this must be refactored to the `ImageCodec` port once task 4.7 lands its service ports.

**4.5 History area and page.** Needs 4.3, 4.4. Owns: `crates/services/src/history.rs`, `crates/services/src/dto/history.rs`, `ui/src/lib/ipc/history.ts`, `ui/src/windows/main/history/` (replacing 1.5's placeholder data), `crates/platform-windows/src/shell.rs` (open, show in folder, recycle, open URL).
- Built: real page/search/get/stats/confirm/delete/copy/show/open requests with read-only repos; native confirm and Recycle Bin deletion; ShellExecute/ShowInFolder; virtual grid/list with paging, selection, keyboard and detail pane. Tests written, not run. The platform shell dependency was later moved behind the `HistoryHost` implemented in the app; services no longer depends on `snapline-win`.

**4.6 Quick panel recents.** Needs 4.5. Owns: the recents and nav count binding in `ui/src/windows/panel/QuickPanel.svelte` and `crates/services/src/app.rs` (`Recent` request).
- Built: `Recent`/`Stats` app requests reading History; QuickPanel real 3-newest thumbnails, duration badges, uploaded icon, exact relative times, empty state and click-to-open; Nav real stats count with focus refresh. Tests written, not run. Last-link footer remains for 6.8.

**4.7 Save pipeline.** Needs 4.1, 4.2, 4.3. Owns: `crates/services/src/saver.rs`, `crates/services/src/worker.rs`.
- Built: `ServicePorts` using core traits, worker thread with the only History write connection and idle-blocking queue, fixed M4 pipeline (copy, save, record, thumbnail, toast, event), atomic file write, naming/counter/subfolder/format/quality/collision handling, best-effort thumbnails and reply channels. Tests written, not run. App wiring (ports factory, worker start, event forwarder) is pending the architecture fix.

### Milestone 5: Annotation

**Goal:** the overlay toolbar and every tool work, undo and redo are exact, and an annotated capture saves exactly as drawn.

**What it needs:**

- The toolbar exactly as section 7.3, shown after selection when the running workflow has an annotate step, and for "Annotate" from History (opens the saved image full screen on the monitor with the cursor, in a non-frozen editor mode where the selection is the whole image).
- Annotations as a list of vector shapes (`Rect`, `Ellipse`, `Arrow` with an optional curve control point, `Pen` polyline smoothed with Catmull-Rom, `Highlight`, `Text`, `Step`, `Blur`), flattened to pixels only on output.
- Every edit is a `Command` (`Add`, `Remove`, `Move`, `Resize`, `Restyle`, `Crop`, `EditText`) that knows how to undo itself.
- A `Canvas` trait with two implementations: Direct2D (live view and final output) and a software rasteriser in `core` (snapshot tests). Both must produce the same result within a tolerance of 2 per channel on anti-aliased edges; the snapshot tests use the software one.
- Text tool: click places a caret; typing uses a native edit box positioned over the shape (an `EDIT` control child of the overlay window, borderless, styled to match), committed on Enter or click outside, cancelled on Esc.
- Step numbers count up from 1 in placement order; removing one renumbers the rest.
- Blur: pixelate in 12 px blocks over the region, computed from the frozen image (never from already-annotated pixels).
- Crop: shrinks the output rect; undoable.
- Colour picker (I): click copies the pixel's hex (`#RRGGBB`) to the clipboard and shows a toast-like pill "Copied #E5484D" for 1.5 s.
- Pin to screen: a small top-most, borderless window showing the image at 100% scale with a 1 px stroke border and shadow, draggable, mouse wheel zooms (25% to 400%), Ctrl+wheel changes opacity (20% to 100%), double-click or Esc closes, right-click menu: Copy, Save as…, Zoom to 100%, Close. Reuses the overlay's Direct2D code.
- Snapshot tests (`insta`) for every shape and style; `proptest` that undo after any edits returns the start state.

**Done when:** snapshot tests pass for every tool; an annotated capture saves exactly as drawn (software and Direct2D outputs compared in a desktop test within the tolerance); every tool works live (checked with Aaron).

**Order:** 5.1 first. Then 5.2, 5.4 together. Then 5.3, 5.5.

**5.1 Annotation model.** Owns: `crates/core/src/annotate/` (`mod.rs`, `shape.rs`, `command.rs`, `document.rs`, `canvas.rs`, `raster.rs`), snapshot files under `crates/core/tests/snapshots/`.
- Built: vector shapes with exact sizes/colours, command history with exact undo/redo and step renumbering, crop, frozen-image pixelation, `Canvas` trait with 2/channel tolerance, software BGRA rasteriser with anti-aliasing, Catmull-Rom pen, deterministic PNG encoder, 20 text snapshots and proptest undo/redo tests. Text uses an in-house 5×7 bitmap glyph set, not DirectWrite; this needs approval and will affect the D2D-vs-software text tolerance in M5. Snapshots are ASCII+FNV text rather than binary PNG; `raster.rs`/`shape.rs` exceed the 400-line guideline.

**5.2 Direct2D canvas and toolbar.** Needs 5.1. Owns: `crates/overlay-windows/src/canvas_d2d.rs`, `toolbar.rs`, `hints.rs`. Shared: `window.rs`.
- Built: Direct2D live/offscreen canvas matching the software rasteriser, 11-tool toolbar with exact metrics and hit testing, tool hints and keyboard pills, annotate/editor document cropping and drawing, and flattened output access. Tests written, not run.

**5.3 Tool interactions.** Needs 5.2. Owns: `crates/overlay-windows/src/tools.rs`, `text_input.rs`. Shared: `window.rs`.
- Built: every tool interaction on the core document (select/move/resize/delete, rect, ellipse, arrow with curve, pen, highlighter, blur, text via a native borderless EDIT, steps, crop, colour sampling with the 1.5 s pill), shape handles, preview drawing, undo/redo and toolbar state, Copy/Save/Pin/Upload flattened output through `OverlayResult.image`, and a one-shot pill timer. Tests written, not run.

**5.4 Pin window.** Needs 5.1. Owns: `crates/overlay-windows/src/pin.rs`. Shared: `window.rs` only if needed.
- Built: one top-most borderless DComp pin thread per call with a retained `PinHandle`; 100% image scale, 1 px border and 12 px layered shadow, drag to move, wheel 25–400%, Ctrl+wheel 20–100% opacity, double-click/Esc close, native right-click Copy/Save as…/Zoom to 100%/Close, capture-location or cursor-monitor placement clamped to the work area, and per-pin window classes. Tests written, not run.

**5.5 Editor entry points.** Needs 5.3. Owns: `crates/services/src/editor.rs` (open from History, open from file, open from clipboard). Shared: `window.rs` (editor mode).
- Built: platform-neutral `EditorService` plus injected host/dialog/clipboard/save/upload/pin ports; opens a saved capture, a PNG/JPEG/WebP/BMP file or a clipboard image/file, runs the non-frozen 100% editor with the whole image selected, routes Copy/Save/Upload/Pin through the existing worker/uploader/clipboard, and cancels cleanly. History Annotate and the Tools Image editor card are wired. Tests written, not run.

### Milestone 6: Networking and uploads

**Goal:** PrtSc puts a working link on the clipboard; a failed upload retries and then shows Retry in History.

**What it needs:**

- HTTP: `ureq` with rustls and `rustls-platform-verifier` (Windows certificate store), proxy from the Proxy setting (system proxy read with `WinHttpGetIEProxyConfigForCurrentUser`), timeout from the setting, `User-Agent: Snapline/<version>`. No async runtime of our own.
- Upload queue on the uploader thread: FIFO, one upload at a time, retries per the setting with backoff 2 s, 8 s, 30 s, 2 min, 5 min (capped at the configured count). The error kind decides: `Retryable` (timeouts, 5xx, 429 with `Retry-After` honoured) retries; `TooLarge` (413), `Auth` (401, 403) and `Permanent` do not.
- `Uploader` trait (section 5.4) and these destinations: Imgur (anonymous upload with Client ID, `POST https://api.imgur.com/3/image`, `Authorization: Client-ID <id>`, deletion through `deletehash`), Amazon S3 and compatible (SigV4 written in-house with `hmac` and `sha2`, `PUT` object, public URL pattern, deletion with the saved ETag), Custom uploaders from ShareX `.sxcu` (import, edit, export, Test upload, JSONPath for link, thumbnail, deletion link and error message; `{json:path}` and `{response}` placeholders as ShareX supports for the URL fields; `{filename}` and `{random}` in the request URL).
- Credentials for each destination read from Credential Manager at upload time, held only for the request.
- EXIF stripping with `img-parts` before any upload when "Remove location and camera data" is on, without re-encoding.
- Link formats for "Copy link": Direct link (`<url>`), Markdown (`![<file name>](<url>)`), HTML (`<img src="<url>" alt="<file name>">`), BBCode (`[img]<url>[/img]`), Page link (the host's page URL when it returns one, else the direct link).
- Upload states in History (`Uploading`, `Link`, `Failed`) with Retry, Upload elsewhere, Cancel (for queued or backing-off uploads), Copy link, Copy deletion link, Delete from host.
- The last link in the quick panel footer, persisted across restarts.
- Upload from clipboard (`upload_clipboard` action and the quick panel): read the clipboard image (or a file path list with one image), ask with a native task dialog when "Ask before uploading from the clipboard" is on ("Upload the image on the clipboard to <destination>?" with "Upload" and "Cancel", and a 160 px preview is not shown, to keep it native and simple), then save it as a capture with source app "Clipboard" and upload it.
- Upload destinations settings page bound to real data (section 7.3), including Import and Export `.sxcu` through native file dialogs in Rust.
- Results return to the UI as `AppEvent::Upload(..)`; the toast "Link copied" appears when notifications are on.

**Done when:** PrtSc (with Upload and Copy link on in the default workflow) puts a working link on the clipboard (checked against a local stub server, and against a real host only if Aaron supplies credentials and says so); a failed upload retries the configured number of times and then shows Retry in History; Retry succeeds against the stub.

**Order:** 6.1, 6.6 together. Then 6.2, 6.3, 6.4, 6.5 together. Then 6.7, 6.8, 6.9.

**6.1 HTTP stack.** Owns: `crates/upload/src/http.rs`, `crates/upload/src/error.rs`, `crates/platform-windows/src/proxy.rs`. Tests: the stub server with a self-signed certificate is rejected by the production verifier and accepted by a test-only verifier built from a fixture CA (the test-only path is behind `#[cfg(test)]`).
- Built: blocking `HttpTransport` over ureq with rustls/platform-verifier, system/custom/no proxy, timeouts, Snapline UA, multipart/form/JSON/raw helpers, typed error mapping and Retry-After; IE proxy detection via dynamic WinHTTP loading; fixture-CA TLS server test with a test-only rustls dev-dependency approved by Aaron. Tests written, not run.

**6.2 Queue.** Needs 6.1. Owns: `crates/upload/src/queue.rs`, `crates/services/src/uploads.rs`. Tests: retry decisions per error kind, backoff schedule, cancel.
- Built: one-at-a-time FIFO uploader thread with uploader thread name, 2/8/30/120/300 s backoff capped by settings, Retry-After override, states Queued/Uploading/BackingOff/Done/Failed/Cancelled, cancel for queued/backing-off and retry for failed/cancelled, typed service DTOs and event sink, core SecretStore bridge. Tests written, not run. App/uploader-thread wiring is pending the destinations registry (6.7).

**6.3 Imgur.** Needs 6.1. Owns: `crates/upload/src/imgur.rs`.
- Built: Client-ID multipart image upload, direct and page links, delete-by-delete-hash, and typed auth/rate-limit/size error mapping. Tests written, not run.

**6.4 S3.** Needs 6.1. Owns: `crates/upload/src/s3/` (`mod.rs`, `sigv4.rs`). Tests: AWS SigV4 test vectors.
- Built: in-house SigV4 canonical request/query/header signing with AWS published vectors, session token and unsigned headers; S3 uploader with virtual-host/path-style/custom endpoints, path prefix, public URL pattern, public-read ACL, ETag capture and If-Match/versionId deletion; typed error mapping and fake transport tests. `snapline-upload` now depends on `snapline-core` (internal). Open point: path-style has no settings control in 7.3.

**6.5 Custom uploaders.** Needs 6.1. Owns: `crates/upload/src/custom/` (`mod.rs`, `sxcu.rs`, `template.rs`). Tests: ShareX example round trips, size and scheme limits, export blanks header values.
- Built: ShareX `.sxcu` import/export with all 7.3 fields, 64 KiB cap, https/http-with-warning and scheme rejection; secret header values stay in Credential Manager and are blank on export unless requested; placeholders `{filename}`, `{random}`, `{json:path}`, `{response}` plus legacy syntax; multipart/binary/form/JSON bodies; JSONPath link/thumbnail/deletion/error. Tests written, not run. JSON/form data uses a base64 file value because the ShareX `Data` field is not in 7.3.

**6.6 EXIF and link formats.** Owns: `crates/upload/src/exif.rs`, `crates/core/src/link_format.rs`.
- Built: all five link formats with HTML attribute escaping and the Page-link fallback; JPEG APP1 EXIF/XMP and PNG eXIf/tEXt/zTXt/iTXt stripping without re-encoding, preserving pixels, ICC and comments; orientation reported rather than silently rotated. Tests written, not run.

**6.7 Destinations page.** Needs 6.3, 6.4, 6.5. Owns: `crates/services/src/destinations.rs`, `crates/services/src/dto/upload.rs`, `ui/src/lib/ipc/upload.ts`, `ui/src/windows/main/settings/UploadsPage.svelte`, `ui/src/windows/main/settings/destinations/`.
- Built: destination registry with credential isolation, Imgur/S3/custom add/edit/test/remove, `.sxcu` import/export, test upload through the queue, and the real Uploads settings page with inline forms and status pills. Tests written, not run.

**6.8 Upload states in History and the quick panel.** Needs 6.2, 6.7. Owns: upload parts of `ui/src/windows/main/history/DetailPane.svelte` and the footer of `QuickPanel.svelte` (coordinate with the integrator: these files were last owned by 4.5 and 4.6, which are finished), `crates/store/migrations/0002_upload_etag.sql` (adds `etag TEXT` to `upload`).
- Built: real Uploaded/Local/Failed/Uploading branches with Copy link, deletion link, host delete, Retry, Cancel and an accessible Upload elsewhere menu; last-link persistence in `uploads.extra["last_link"]` and no-scheme display with None yet/Copy disabled; migration 0002 adds `etag`; queue states now carry a capture id and the worker persists them into History so failed/queued rows survive and show Retry. Tests written, not run.

**6.9 Upload from clipboard.** Needs 6.2. Owns: `crates/services/src/clipboard_upload.rs`, `crates/platform-windows/src/task_dialog.rs`.
- Built: clipboard image and one-image path-list source, optional native confirmation, codec/worker save as a Clipboard capture, upload enqueue and toast. Tests written, not run.

### Milestone 7: Workflows

**Goal:** workflows are data; one runner executes any of them; each can have its own hotkey; the five design examples run from their hotkeys.

**What it needs:**

- `Workflow { id, name, hotkey: Option<Chord>, enabled, steps: Vec<Step> }` stored in `settings.toml` as `[[workflow]]` tables. `Step` is one of: `Capture { kind: Region | Window | Screen | Scrolling | LastRegion }`, `Record { kind: Video | Gif | Window }`, `CopyImage`, `Annotate`, `Save`, `Upload { dest: Option<DestId> }`, `CopyLink { format }`, `ExtractText`, `CopyText`, `ConvertToGif`, `Notify`, `PlaySound`.
- `run(&Workflow, &Ports, &mut RunContext) -> Result<RunReport, RunError>`: each step reads and writes the context (image, saved file, link, text). Upload steps hand off to the queue and the runner continues with the steps that don't need the link; steps that need the link (`CopyLink`) run when the upload finishes, through `AppEvent::Upload(Uploaded)` matched by run id. A workflow never blocks on the network.
- Steps that need later milestones (`Record`, `ConvertToGif`, `ExtractText`, `Scrolling`) go through ports that report "Not available yet" until M8 and M9 fill them in; a workflow containing one shows a toast "<step> is not available yet" and stops before it.
- Migration: the default workflow (id `default`, name "Screenshot to link") is created from the General page's default-steps list; the settings version increases (the integrator assigns it).
- Workflow chords join the fail-closed plan; disabled workflows don't claim their chord. The default workflow has no chord of its own: it runs from the Capture region action.
- Workflows tab bound to real data, the editor (section 7.3), and the General page's default workflow section editing the `default` workflow.
- The five examples as presets, created on first run (not on upgrade if the user already has workflows): Screenshot to link (default); Annotate and share (Ctrl+Shift+A, on: Capture region, Annotate, Upload, Copy link as Markdown); Bug report clip (Ctrl+Alt+R, on: Record region, Convert to GIF, Upload to the Videos and GIFs default, Copy link); Grab text (Ctrl+Alt+T, off: Capture region, Extract text, Copy text); Save quietly (Ctrl+Alt+PrtSc, off: Capture screen, Save).

**Done when:** "Screenshot to link", "Annotate and share" and "Save quietly" run from their hotkeys; "Bug report clip" is checked in M8 and "Grab text" in M9; the default set has no internal clash with every workflow enabled (test).

**Order:** 7.1 first (merge its types early). Then 7.2, 7.4 together. Then 7.3, 7.5. Then 7.6.

**7.1 Workflow model and runner.** Owns: `crates/core/src/workflow/` (`mod.rs`, `step.rs`, `run.rs`). Tests with fake ports for every step and for failure part-way through.
- Built: `Workflow`/`Step`/validation with typed UI strings, workflow-specific ports for capture/record/annotate/save/upload/text/GIF, `run` and `resume_after_upload`, deferred CopyLink, upload handoff by run id, typed not-available steps, 40 fake-port tests written not run. Validation strings and the default workflow creation are left to 7.2/7.6.

**7.2 Workflows in settings.** Needs 7.1. Owns: `crates/store/src/settings/workflows.rs` and its migration in `migrate.rs` (shared with the integrator for the version number).
- Built: settings version 2 with a real workflow list (core `Workflow`/`Step` types), unknown keys preserved, and a 1→2 migration that adds one default workflow only when the list is empty. Default steps are Copy image, Save, Upload, Copy link Direct, Play sound; Annotate is omitted because the model has no per-step enabled flag. Tests written, not run. `store` now has the inward `snapline-core` dependency. The default workflow has no Capture step and needs the 7.4/7.3 validation exception.

**7.3 Workflow hotkeys and running.** Needs 7.1, 7.2. Owns: `crates/services/src/workflows.rs`. Shared: the chord collection function in `crates/services/src/hotkeys.rs`.

**7.4 Workflows tab and editor.** Needs 7.1's types. Owns: `crates/services/src/dto/workflow.rs`, `ui/src/lib/ipc/workflow.ts`, `ui/src/windows/main/workflows/`.
- Built: typed List/Get/Create/Update/Delete/Duplicate/Validate DTOs with a `StepDto` wrapper matching the core serde shape; services persistence, default guards, validation mapping and hotkey replanning; real workflow cards with clash text and the full editor with reorder/parameters/duplicate/delete. Tests written, not run. Upload-destination options await 6.7's destination identifiers.

**7.5 Default workflow in Settings.** Needs 7.2, 7.4. Owns: the default workflow section of `ui/src/windows/main/settings/GeneralPage.svelte`.

**7.6 Presets.** Needs 7.3. Owns: `crates/core/src/presets.rs`. Tests: no internal clash with everything enabled.

### Milestone 8: Recording

**Goal:** MP4 and GIF recording of a region or window, with system audio and mic, costing nothing when not recording.

**What it needs:**

- Bundled FFmpeg: `cargo xtask fetch-ffmpeg` downloads a pinned GPL build for Windows x64 (the URL, version and SHA-256 go in `xtask/ffmpeg.lock`; Aaron chooses the build source before task 8.1 starts), checks the hash, and unpacks `ffmpeg.exe`, its licence and a `SOURCE.txt` with the matching source link into `third_party/ffmpeg/`.
- `ffmpeg.exe` from the install folder (or the user's chosen absolute path), arguments as a list, never through `PATH` or a shell.
- Video capture with FFmpeg's `ddagrab` filter (GPU desktop duplication) cropped to the region or window, encoded with the GPU's hardware encoder when present (`h264_nvenc`, `h264_qsv`, `h264_amf`, or `hevc_*` for H.265; detected once per app run with `ffmpeg -hide_banner -encoders` and a 1-frame test encode), otherwise `libx264`/`libx265`/`libvpx-vp9`. Quality presets map to: High = CQ/CRF 20, Balanced = 26, Small file = 32, Lossless = lossless mode of the encoder.
- GIF: record to a temporary lossless MP4, then convert with `palettegen` (when "Optimise colour palette" is on, `stats_mode=diff`) and `paletteuse` at the GIF frame rate and maximum width.
- System audio (WASAPI loopback on the default render device) and microphone (WASAPI capture on the chosen device) captured in Rust and piped to FFmpeg as raw PCM through named pipes (`\\.\pipe\snapline-audio-<n>`), mixed by FFmpeg (`amix`).
- Recording border: a click-through (`WS_EX_TRANSPARENT | WS_EX_LAYERED`), top-most window drawing a 2 px red (`annotation.red`) frame just outside the recorded area, excluded from capture with `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`.
- Recorder thread only while recording, driven by a state machine in `core`: `Idle → Arming → Recording → Stopping → Encoding → Done | Failed`.
- Stopping: the record hotkey again, the tray menu's "Stop recording", or the tray icon left-click while recording. Stop sends `q` to FFmpeg's stdin, waits up to 5 s, then kills it.
- Tray: the recording icon, tooltip with elapsed time (updated once per second only while recording), the Record submenu's System audio and Microphone check items bound to the Capture settings.
- A finished file is saved through the M4 pipeline and appears in History with type, size, duration and a thumbnail from the first frame (FFmpeg `-frames:v 1`).
- Fills the workflow ports for `Record` and `ConvertToGif`, then checks "Bug report clip".

**Done when:** a 60 s 1080p30 recording stays under 150 MB (SnapLine plus FFmpeg) and under 5% CPU with a hardware encoder (budget run with Aaron); "Bug report clip" works from its hotkey.

**Order:** 8.1, 8.2 any time. Then 8.3, 8.4, 8.5 together. Then 8.6. Then 8.7.

**8.1 Fetch FFmpeg.** Owns: `xtask/src/ffmpeg.rs`, `xtask/ffmpeg.lock`.
- Built: pinned gyan.dev essentials GPL 9.0.2 (2026-09-19) with SHA-256 `60f46726…2047ba`; download via ureq IPv4-first, hash check, `tar.exe` zip extraction, idempotent install into `third_party/ffmpeg` with `ffmpeg.exe`, `LICENSE`, `SOURCE.txt` and an archive hash stamp. Seven tests written, not run. Fetch succeeded and a second run was a no-op.

**8.2 Recording logic.** Owns: `crates/core/src/recording.rs`, `crates/core/src/ffmpeg_args.rs`. Tests for every state change and every argument set.
- Built: `RecordingMachine` with Idle/Arming/Recording/Stopping/Encoding/Done/Failed, events, elapsed/target/output/error and a 5 s stop timeout constant; FFmpeg command builders for video (ddagrab, hardware candidates, quality presets, formats/fps, audio pipes and amix), GIF capture/convert, converter, encoder detection and one-frame probe. 54 tests written, not run. Encoder-specific flags and PCM defaults are implementation choices where the plan was silent.

**8.3 FFmpeg runner.** Needs 8.1. Owns: `crates/platform-windows/src/ffmpeg.rs` (spawn by absolute path with `CREATE_NO_WINDOW`, encoder detection, progress from `-progress pipe:1`, stop and kill).

**8.4 Audio.** Owns: `crates/platform-windows/src/audio.rs` (device list for the Microphone device select, loopback and capture, PCM to named pipes).
- Built: WASAPI shared-mode loopback on the default render device and capture on the chosen/default microphone; mix-format conversion to s16le/f32le/s32le 48 kHz stereo (or caller format), silence padding, friendly device names, named pipes `\\.\pipe\snapline-audio-<n>`, stop with CancelSynchronousIo/join cap, full RAII and close event classification. Tests written, not run. Local kernel32 declarations/property-store vtable used instead of adding missing Windows features.

**8.5 Recording border and target pick.** Owns: `crates/overlay-windows/src/record_border.rs`. Shared: `window.rs` (a record mode for the picker that skips the toolbar).
- Built: click-through layered top-most 2 px red border outside the recorded area, clamped to the monitor and excluded from capture with `WDA_EXCLUDEFROMCAPTURE`; record pick mode with drag-region or click-window results (rect, monitor, window handle/title), no toolbar/hints, and existing selection/snap/magnifier. Tests written, not run.

**8.6 Recorder wiring.** Needs 8.2 to 8.5. Owns: `crates/services/src/recording.rs`, `crates/services/src/dto/record.rs`, `ui/src/lib/ipc/record.ts`. Shared: `apps/snapline/src/tray.rs` (recording state), Capture settings bindings for the recording rows.
- Built: real recorder service/thread over the core state machine, region/window pick through record mode, live 2 px border, FFmpeg ddagrab + hardware fallback, WASAPI system/mic pipes, stop via hotkey/tray/left-click with `q` + 5 s kill, first-frame thumbnail and History save through a worker command (the worker keeps the only History write connection), per-second tray icon/tooltip, settings-bound audio checks, typed Record DTO/IPC/events, and real Record/ConvertToGif workflow ports. Tests written, not run.

**8.7 Recording budget.** Needs 8.6. Owns: `xtask/src/budget.rs` (adds the 60 s 1080p30 run).
- Built: opt-in `cargo xtask budget --record` starts the app in budget mode, records via the normal region recorder, validates a 1920x1080@30 hardware-encoder FFmpeg command line, samples six 10 s windows, checks under 150 MiB and under 5% CPU, stops cleanly and reports each failure. The run is interactive: Aaron must select a 1920x1080 region when the overlay appears. Tests written, not run; the real 60 s run waits for Aaron.

### Milestone 9: Tools

**Goal:** every card on the Tools tab works from the card and from its hotkey.

**What it needs:**

- OCR: `Windows.Media.Ocr` (`OcrEngine::TryCreateFromUserProfileLanguages`), using the installed language packs. The tool opens the overlay without the toolbar, the user selects an area, and the recognised text (lines joined with `\r\n`) goes to the clipboard with a toast "Copied <n> lines of text". The text is also stored in `capture.ocr_text` when run on a saved capture so search finds it. If no OCR language is installed, toast "Windows has no text recognition language installed. Add one in Settings › Time & language › Language & region." Fills the workflow's `ExtractText` port; check "Grab text".
- Scrolling capture: the user picks a window or region; SnapLine scrolls it with `SendInput` mouse-wheel events (one notch per step) over the centre of the area, waits the configured delay, captures, and repeats until two captures in a row are identical, the maximum count is reached, or the user presses Esc. Stitching in `core` matches overlapping rows (compare row hashes, find the largest overlap of at least 20 rows); with "Remove sticky headers" on, rows that repeat at the same position in every capture (top and bottom bands) are kept only once. The result goes through the normal save pipeline.
- Screen ruler: a native overlay over the monitor with the cursor; drag to measure; shows width, height, diagonal length and angle in physical pixels in a readout pill; Shift locks to horizontal or vertical; Esc closes; Ctrl+C copies "W × H".
- Colour picker tool: the overlay's magnifier without selection; click copies the colour in the format chosen in a small format switch in the magnifier (Hex `#RRGGBB`, RGB `rgb(r, g, b)`, HSL `hsl(h, s%, l%)`; the last choice is remembered); Esc cancels.
- QR code (`#/tools/qr`): two tabs "Scan" and "Make". Scan: opens the overlay to select an area, decodes with `rqrr`, shows the text with Copy and (for `http(s)` links) Open. Make: a text field, a live QR preview (SVG from Rust, `qrcode` crate, error correction M), Copy image and Save as PNG.
- Image editor card: native file dialog (PNG, JPEG, WebP, BMP), then the M5 editor.
- Pin to screen card: a native popup menu "From file…" or "From clipboard", then the M5 pin window.
- Image combiner (`#/tools/combine`): pick captures from History (a picker grid using `Thumb`) or files; order by drag; layout Side by side or Stacked (segmented); Spacing (range 0 to 64 px, default 8); Align (Start, Centre, End); Background (Transparent, White, Black, or a hex field). Preview from Rust as a downscaled PNG through a `combine` URI scheme (same rules as `thumb`: a numeric job id only). Save adds the result to History.
- Video converter (`#/tools/convert`): pick a video from History or a file; output GIF, WebM (VP9) or MP4 (smaller, CRF 28); progress bar from FFmpeg's progress output; Cancel; the result goes to History.
- GitHub Gist destination (text and code): token in Credential Manager, public or secret gist, file name from the capture name, returns the gist URL. Used for "Text and code" uploads (OCR text, copied text).
- SFTP destination: `ssh2` with password or private key (the key file path is stored in settings; its passphrase, if any, in Credential Manager), host key checked against `%APPDATA%\SnapLine\known_hosts` (trust on first use with a confirm dialog showing the SHA-256 fingerprint), remote folder, public URL pattern.
- The highlighter's text-line snapping from the design (deferred from M5), using OCR line boxes: when the highlighter is dragged over a line that OCR found, the stroke snaps to that line's box. Only if Aaron confirms he still wants it.

**Done when:** each tool works from its card and its hotkey (where it has one), checked with Aaron; "Grab text" works from its hotkey; Gist and SFTP pass their stub tests.

**Order:** pure maths for 9.2 (stitching), 9.3 (ruler) and 9.7 (combiner layout) can start any time. Then 9.1, 9.4, 9.5, 9.9, 9.10 together. Then 9.2, 9.3, 9.6, 9.7, 9.8.

**9.1 OCR.** Owns: `crates/platform-windows/src/ocr.rs`, `crates/services/src/tools/ocr.rs`.
- Built: Windows.Media.Ocr engine with installed language packs, BGRA→SoftwareBitmap conversion, `\r\n` line joining, exact no-language message, per-thread WinRT apartment; services tool copies text, returns `Copied <n> line(s) of text`, optionally stores OCR text, and implements the workflow `TextPort`. Tests written, not run. Missing: a `HistoryRepo::set_ocr_text` method and `ipc_tool` OCR variants for the M9 UI.

**9.2 Scrolling capture.** Owns: `crates/core/src/stitch.rs`, `crates/platform-windows/src/scroll.rs`, `crates/services/src/tools/scrolling.rs`. Tests: stitching on synthetic images with and without sticky headers.
- Built: row-hash overlap stitching with 20-row minimum, sticky top/bottom band removal, duplicate/max-height/limit/Esc stop reasons; SendInput wheel with cursor-centred area and Esc detection; services orchestration with injectable capture/scroller/waiter and settings mapping. Tests written, not run. `DEFAULT_MAX_HEIGHT = 16_384` is the only invented value.

**9.3 Screen ruler.** Owns: `crates/core/src/ruler.rs`, `crates/overlay-windows/src/ruler.rs`, `crates/services/src/tools/ruler.rs`. Shared: `window.rs`.

**9.4 Colour picker.** Owns: `crates/core/src/colour.rs`, `crates/overlay-windows/src/picker.rs`, `crates/services/src/tools/colour.rs`. Shared: `window.rs`.

**9.5 QR codes.** Owns: `crates/core/src/qr.rs`, `crates/services/src/tools/qr.rs`, `ui/src/windows/main/tools/QrTool.svelte`.
- Built: error-correction-M SVG/Image generation, BGRA luma scan with `rqrr`, URL detection; services make/copy/save/scan; Tool DTOs and QrTool Scan/Make tabs with live preview and exact actions. Tests written, not run. Per Aaron, QR is parked for now; Scan/copy-text/save-PNG remain `NotYet` until the overlay selection and file dialogs are wired.

**9.6 Image editor and pin cards.** Owns: `crates/services/src/tools/editor.rs`, `crates/services/src/tools/pin.rs`. Shared: `window.rs`.
- Built: Pin to screen card with the native Win32 From file…/From clipboard popup (`snapline_win::menu`), editor-service pin helpers, image decoding and the existing retained `EditorPin` handles; `ToolResp::Available` reports the editor and pin cards from the installed ports. Tests written, not run.

**9.7 Image combiner.** Owns: `crates/core/src/combine.rs`, `crates/services/src/tools/combine.rs`, `ui/src/windows/main/tools/CombinerTool.svelte`, the `combine` scheme in a new file `apps/snapline/src/protocol_combine.rs` (registered by the integrator in `main.rs`).
- Built: pure side-by-side/stacked layout with spacing/align/background and no upscaling; process-local preview job registry with numeric ids only; `combine` scheme with correct MIME/404; CombinerTool with History picker, file picker, reorder, options, preview and save. Scheme registered in main and CSP updated. Tests written, not run. Per Aaron, the combiner is parked for now.

**9.8 Video converter.** Needs 8.2, 8.3. Owns: `crates/services/src/tools/convert.rs`, `ui/src/windows/main/tools/ConverterTool.svelte`. Shared: new builders in `crates/core/src/ffmpeg_args.rs`.

**9.9 Gist.** Owns: `crates/upload/src/gist.rs`, its form in `ui/src/windows/main/settings/destinations/GistForm.svelte`.
- Built: GitHub Gist uploader with Bearer token from Credential Manager, public/secret gist, raw/page links, typed error mapping and no token leakage; GistForm with SecretInput/Public/Test/Save/Remove, plus registry and UploadsPage routing as a real destination. Tests written, not run.

**9.10 SFTP.** Owns: `crates/upload/src/sftp.rs`, `crates/platform-windows/src/known_hosts.rs`, `ui/src/windows/main/settings/destinations/SftpForm.svelte`.
- Built: real SFTP destination with password/private-key auth, remote folder, public URL pattern, TOFU host-key store and fingerprint prompt, registry/UploadsPage routing, secret isolation and test upload through the queue. App injects the Windows known-hosts store and TaskDialog confirmer. Tests written, not run.

Every tool task also replaces its card's disabled state on the Tools page (`ToolsPage.svelte` reads availability from `ipc_tool` → `Available`, so no task edits the page itself) and adds its `ipc_tool` request variants to `crates/services/src/dto/tool.rs` (one task at a time; the integrator orders them).

### Milestone 10: Release

**Goal:** a clean PC installs SnapLine without admin rights, every milestone's checks pass, and it uninstalls cleanly.

**What it needs:**

- `cargo xtask package`: builds the UI and a release binary, then runs the Tauri bundler for an **NSIS installer** with `installMode: "currentUser"` (installs to `%LOCALAPPDATA%\Programs\Snapline` without admin rights), `webviewInstallMode` `embedBootstrapper` (installs the WebView2 Evergreen runtime only if missing, which can happen on Windows 10), and `resources` containing `ffmpeg.exe`, the GPLv3 text, FFmpeg's licence and `SOURCE.txt`. NSIS is used instead of MSI because Tauri's MSI target is built on WiX for per-machine installs, while its NSIS target supports per-user installs, the WebView2 bootstrapper and uninstall hooks directly.
- The installer creates a Start menu shortcut with the AppUserModelID `io.github.aaronburt.snapline` (for toasts) and offers "Launch Snapline" at the end.
- A portable zip with the same files plus an empty `portable` marker file.
- Uninstall cleanup through an NSIS uninstall hook (`NSIS_HOOK_PREUNINSTALL`) that runs `snapline.exe --uninstall-cleanup`: remove the Run key value, restore the PrtSc setting from `[hotkeys.restore]`, remove the AUMID registry key, delete Credential Manager entries named `SnapLine:*` only if the user agrees, keep captures, and ask "Also delete Snapline's settings and history?" (No by default).
- `cargo xtask budget` enforced in `ci` for every row of section 6.
- Accessibility pass: every web page passes `axe-core` with no violations in vitest; full keyboard use (Tab order matches visual order, every control reachable, Esc closes popovers); screen reader labels on icon-only buttons; the overlay announces the selection size through UI Automation (`UiaRaiseNotificationEvent`) and its toolbar is reachable with Tab when it is shown.
- Testing on Windows 10 22H2 and Windows 11 (clean install, every milestone's done-when checks, clean uninstall).

**Done when:** a clean PC installs SnapLine without admin rights, runs every milestone's checks, and uninstalls cleanly; `cargo xtask ci` passes including all budgets.

**Order:** 10.1 and 10.2 together. Then 10.3. Then 10.4 alone (it touches every page). Then 10.5.

**10.1 Installer and portable zip.** Owns: `xtask/src/package.rs`, the `bundle` section of `apps/snapline/tauri.conf.json` (integrator-approved), `apps/snapline/installer/` (NSIS hooks). Shared: portable path lookup in `crates/store/src/io.rs`.
- Built: NSIS currentUser bundle with embedBootstrapper and FFmpeg/licence/source resources; install path `%LOCALAPPDATA%\Programs\Snapline`, AUMID shortcut, launch-at-finish, pre-uninstall cleanup hook; `cargo xtask package` builds UI + NSIS and creates the portable zip with a `portable` marker using `tar.exe`. Config/hook/staging tests written, not run; NSIS not compiled because `makensis` is unavailable.

**10.2 Uninstall cleanup.** Owns: `crates/services/src/uninstall.rs`. Shared: `--uninstall-cleanup` in `apps/snapline/src/cli.rs`.
- Built: `cleanup` removes the Run value, restores PrtSc, removes the AUMID, lists/deletes only `SnapLine:*` credentials, asks before deleting credentials/settings/history, keeps captures, and reports each step; `--uninstall-cleanup` runs it from the NSIS hook. Tests written, not run.

**10.3 Full budget check.** Owns: `xtask/src/budget.rs`.

**10.4 Accessibility pass.** Runs alone. Touches every `.svelte` file and the overlay's toolbar code.
- Built: axe-core checks for every page/component; accessible names and labels, focus-trapped popovers/menus with Esc and outside close, keyboard segmented controls and menus, live status/error regions, named progress bars, table/list semantics, thumbnail alt text and disabled reasons; overlay selection UIA notification and a Tab-reachable toolbar with a visual focus ring and announcements. Tests written, not run. Narrator/NVDA delivery and real WebView2 focus order still need a human check.

**10.5 Windows 10 and 11 test pass.** Last. Needs Aaron's screen or a virtual machine.

---

## 11. Order of work at a glance

| Step | Tasks |
|------|-------|
| 1 | 0.1 |
| 2 | 0.2, 0.3, 0.4 |
| 3 | 0.5, then milestone 0 commit |
| 4 | 1.0 alone |
| 5 | 1.1, 1.2, 1.3, 1.8 (1.1 is the gate: stop if it fails) |
| 6 | 1.4, 1.5, 1.6, 1.7, then 1.9, then milestone 1 commit |
| 7 | 2.1, 2.3, 2.6, 2.7, 2.8, then 2.2, 2.4, then 2.5, then milestone 2 commit |
| 8 | 3.1, 3.2, 3.5, then 3.3, then 3.4, 3.6, then milestone 3 commit |
| 9 | 4.1, 4.2, 4.3, then 4.4, 4.7, then 4.5, 4.6, then milestone 4 commit |
| 10 | 5.1, then 5.2, 5.4, then 5.3, 5.5, then milestone 5 commit |
| 11 | 6.1, 6.6, then 6.2 to 6.5, then 6.7, 6.8, 6.9, then milestone 6 commit |
| 12 | 7.1, then 7.2, 7.4, then 7.3, 7.5, then 7.6, then milestone 7 commit |
| 13 | 8.1, 8.2 (can start any time), then 8.3, 8.4, 8.5, then 8.6, then 8.7, then milestone 8 commit |
| 14 | 9.x as in milestone 9's order, then milestone 9 commit |
| 15 | 10.1, 10.2, then 10.3, then 10.4, then 10.5, then milestone 10 commit |

A single agent runs the tasks in each step one after another, left to right.

---

## 12. Questions for Aaron before the milestone that needs them

The executing agent must not answer these itself.

| Needed by | Question |
|-----------|----------|
| 1.1 | If the webview gate fails even with fallback A: switch to fallback B (a separate UI process), or accept the extra idle memory? |
| 1.2 | If muda can't draw dark tray menus: draw the menu natively instead? |
| 2.8 | Shutter and "done" sound files (WAV), or keep the Windows system sounds? |
| M2 (hotkeys) | Keep the design's Ctrl+Shift+letter defaults that take shortcuts from other apps (section 7.5), or move tools and workflows to other chords? |
| 6 | Real Imgur or S3 credentials for a live check, or stub server only? |
| 8.1 | Which FFmpeg build source and version to pin (for example a GPL "essentials" build for Windows x64)? |
| 9 | Keep the highlighter's text-line snapping from the design? |
| 10 | NSIS per-user installer instead of the original per-user MSI: OK? |

## 13. Deferred

- AVIF output (future issue, 2026-11-09).
- Imgur account sign-in (OAuth); anonymous uploads with a Client ID only.
- Check for updates and auto-update; code signing. Both wait for a public remote.
- Views in the design file that are not in the v2 navigation: Uploads, Recordings, Text & OCR.
- Pastebin and is.gd destinations shown in the design's option lists.
- Linux build: later, separate, GNOME-style design (`platform-linux`, `overlay-linux`, a GTK4/libadwaita or webview front end, `apps/snapline-linux`), reusing `core`, `store`, `upload` and `services` unchanged.

## 14. Status

| # | Milestone | Status | Commit |
|---|-----------|--------|--------|
| 0 | Foundations | Built, not tested | |
| 1 | Basic UI shell (includes the webview gate) | Built, not tested | 6ff3546 |
| 2 | Settings and hotkeys | Built, not tested | e92db22 |
| 3 | Screenshot | Built, not tested | 574495e |
| 4 | Saving to disk and History | Built, not tested | 574495e |
| 5 | Annotation | Built, not tested | ad3e7f7 |
| 6 | Networking and uploads | Built, not tested | 1ed0d15 |
| 7 | Workflows | Built, not tested | 175ff93 |
| 8 | Recording | Built, not tested | 4a732fe |
| 9 | Tools | Built, not tested | 4b7b6d7 |
| 10 | Release | Built, not tested | d2571fa |

Update this table, and the task's line, whenever a task or milestone lands.
