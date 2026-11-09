# Snapline bugs

Status: every item below looks fixed in the working tree (code checked, not yet confirmed in the running app). Delete this file once confirmed.

## 1. New capture button does nothing

- File: `ui/src/windows/main/Nav.svelte`
- `newCapture()` is `void 0;`.
- The button never dispatches the `capture_region` action.

## 2. Overlay selection and dimming are invisible

- File: `crates/overlay-windows/src/window.rs`
- `WindowState` stores only `swap_chain` and `target`.
- The `SwapTarget` returned by `create_target` also owns `_dcomp_target` and `_visual`. Those fields are dropped immediately.
- Dropping them destroys the DirectComposition visual tree, so the swap chain renders into nothing.

## 3. Capture toolbar and menus do not appear

- File: `crates/services/src/capture.rs`
- The overlay request is always built with `annotate: false`.
- `make_layout` only shows the toolbar when `scene.annotate && scene.document.is_some()` and the state is `Annotating` or `Adjusting`.
- The workflow's Annotate step is never communicated to the overlay, so enabling the Annotate step would still not show the toolbar.
- The hint and keyboard pills are also affected by bug 2.

## 4. Title bar cannot move the window

- File: `ui/src/windows/main/TitleBar.svelte`
- `data-tauri-drag-region` is only on the outer `.titlebar`.
- Child elements (logo, name, search wrapper, spacer) intercept the mouse.
- There is no explicit `getCurrentWindow().startDragging()` handler.

## 5. History does not refresh automatically

- File: `ui/src/windows/main/history/HistoryPage.svelte`
- The page only loads on mount and when the filter or search changes.
- It never subscribes to `onEvent` for history added/removed or terminal upload state changes.
- `DetailPane.svelte` already listens to events, so the event path works; the page ignores them.

## 6. Captures end with an upload error

- The default workflow always includes `Upload { dest: None }` and `CopyLink`.
- Default settings use `uploads.images = "save_locally_only"`.
- `QueueUploadHandOff` rejects that with: `Choose an upload destination for images in Settings.`
- The capture is still saved, but the workflow reports Step 4 as failed.

## 7. General default-workflow settings are not used by the runner

- The General page's `general.default_workflow` toggles are stored separately.
- The runner reads `workflows.list` and falls back to the core preset.
- In the current portable `settings.toml`, `[workflows]` is empty, so enabling "Open in editor to annotate" in General has no effect on the actual capture workflow.

## 8. History Pin and Upload actions are stubs

- File: `ui/src/windows/main/history/HistoryPage.svelte`
- `pin(_id)` is `void _id;`.
- `upload(_id)` is `void _id;`.
- Both buttons do nothing.

## 9. History count in the sidebar can go stale

- File: `ui/src/windows/main/Nav.svelte`
- The count refreshes only on mount and window focus.
- It does not subscribe to history events, so captures added while the window stays focused will not update the count until focus changes.
