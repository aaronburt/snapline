# Snapline

A lightweight Windows tray app for screenshots and screen recordings. Capture a region, window or screen, annotate it, save it, upload it and copy the link.

- Screenshots and recordings (video or GIF through FFmpeg)
- Annotation tools
- Uploads to S3-compatible storage, SFTP, GitHub Gist and custom `.sxcu` destinations
- Capture history, global hotkeys and a tray Quick Panel

Windows 10 or 11 (x64) only. Releases are on the GitHub releases page; the installer is not code signed, so SmartScreen may warn.

## Build

Needs the Rust toolchain in `rust-toolchain.toml`, and NSIS (`winget install NSIS.NSIS`) for the installer.

```
cargo xtask fetch-ffmpeg
cargo build -p snapline
cargo xtask package
```

`package` builds the release exe, the NSIS installer and a portable zip.

## Test

```
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Licence

GPL-3.0-or-later. See `LICENSE`.
