# ADR 0006: Svelte frontend and the IPC contract

Status: Accepted

## Context

The web UI needs a small renderer heap, strict typing across the boundary, no remote code, and a contract that lets feature tasks work independently. The v2 boards are HTML with repeated inline styling and icon paths.

## Decision

The frontend is Svelte 5 with runes, TypeScript in strict mode, Vite, plain CSS and no router or component library. Rust is the single source of truth. There is one Tauri command per area, `ipc_app`, `ipc_settings`, `ipc_hotkeys`, `ipc_history`, `ipc_upload`, `ipc_workflow`, `ipc_record` and `ipc_tool`; each takes that area's request enum and returns its response enum or `IpcError`. Request and response types live in `crates/services/src/dto`, derive `ts_rs::TS`, and are generated into `ui/src/lib/ipc/bindings` by `cargo xtask bindings`. One event name, `snapline://event`, carries the `AppEvent` enum. Hand-written wrappers in `ui/src/lib/ipc` expose one typed function per request. Tokens are generated from `design/tokens.toml` into CSS custom properties and an overlay palette. The webview only loads bundled files, has no browser storage that matters and makes no network requests. Capabilities expose only the eight commands, event listening and the title bar window controls.

## Consequences

Rust owns application state and survives every window closing. Generated bindings must be committed and checked in CI. Adding an area variant touches one DTO file and one handler. The frontend cannot use a plugin such as fs, shell, dialog or http; file dialogs, clipboard, opening files and URLs are Rust requests. Svelte keeps the renderer small and a virtual list is required for history above 60 rows.
