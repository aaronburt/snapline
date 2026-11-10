# Snapline

Snapline is a ShareX alternative for Windows: a lean tray app that captures screenshots and screen recordings, annotates them, saves them, uploads them to image hosts or your own server, and copies a link. It is open source under GPLv3-or-later. Build the web frontend with `cargo xtask ui`, then build the app with `cargo build`.

## Coverage

For Rust coverage, install the tools with `cargo install cargo-llvm-cov --locked` and `rustup component add llvm-tools-preview`, then run `cargo llvm-cov --workspace --html`. The report is written to `target/llvm-cov/html/index.html`. Desktop tests marked ignored are excluded from the default run.

For frontend coverage, run `npm ci` and `npm run coverage` in `ui`. The summary appears in the terminal and the HTML report is written to `ui/coverage/index.html`. Generated IPC bindings and test files are excluded.
