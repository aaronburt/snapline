# Snapline

Snapline is a screenshot and screen recording tool for Windows, built as a lean tray app. It captures screenshots and recordings, annotates them, saves them, uploads them to image hosts or your own server, and copies the link. It is open source under GPL-3.0-or-later.

## Features

- Screenshot capture of a region, a window or a full screen, with a native selection overlay.
- Screen recording to video or GIF through FFmpeg, including on secondary monitors.
- Annotation tools drawn natively.
- Uploads to Imgur, S3-compatible storage, SFTP, GitHub Gist and custom `.sxcu` destinations, with a copied link.
- A history of captures with thumbnails and the last uploaded link.
- A Quick Panel opened from the tray icon, global hotkeys and native Windows notifications.

## Requirements

- Windows 10 or 11 (x64). Snapline is Windows only.
- The Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it on first use).
- Node.js and npm for the web frontend in `ui`.
- The WebView2 runtime, which ships with current Windows.
- FFmpeg, which `cargo xtask fetch-ffmpeg` downloads into `third_party/ffmpeg`.
- NSIS, which Tauri fetches on the first installer build.

## Build and run

```
cd ui
npm ci
npm run build
cd ..
cargo xtask fetch-ffmpeg
cargo build -p snapline
target\debug\snapline.exe
```

Build the installer and portable zip with:

```
cargo xtask package
```

The unsigned installer is written to `target/release/bundle/nsis` and the portable zip to `target/portable`.

## GitHub releases

After a pull request merges into `main` and the merged commit passes CI, GitHub Actions publishes a Windows x64 release with the app executable, the unsigned NSIS installer, and a portable ZIP containing the executable and its runtime files.

Release tags start at `v0.1` and increase the minor number for each release. The minor number keeps growing past `0.999` to `0.1000` and beyond. Windows package metadata uses a three-part version such as `0.1.0`. Above minor 65535 it uses `0.65535.65535` to fit Windows version fields, while the GitHub release and download filenames retain the full version.

## Tests and checks

```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo deny check
cargo xtask no-comments
cd ui
npm test
npm run check
```

`crash::tests::panic_child_process` is a helper for the crash report test and can fail when run as part of the whole workspace. Skip it with `cargo test --workspace -- --skip crash::tests::panic_child_process`.

For coverage, install `cargo-llvm-cov` and run `cargo llvm-cov --workspace --html`, or run `npm run coverage` in `ui`.

## Project layout

- `apps/snapline`: the Tauri app, tray, windows and IPC.
- `crates/core`: shared types, actions, geometry and ports.
- `crates/overlay-windows`: native Direct2D overlays, toolbar, pin, ruler and recording border.
- `crates/platform-windows`: Windows capture, hotkeys, shell and input.
- `crates/services`: application services, workflows, recording and uploads.
- `crates/store`: settings and the history database.
- `crates/upload`: upload destinations.
- `ui`: the Svelte frontend.
- `xtask`: build, packaging and check tasks.
- `docs/adr` and `docs/plans`: design decisions and plans.

## Known limits

- Windows only.
- The installer is not code signed, so Windows SmartScreen may warn.

## Licence

GPL-3.0-or-later. See `LICENSE`.
