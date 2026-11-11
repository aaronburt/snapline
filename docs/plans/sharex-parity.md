# Snapline: ShareX parity plan for media processing and sharing

Written: 2026-11-10. Revised: 2026-11-10 after review. Owner: Aaron. Status: proposal, nothing built.

This is the roadmap for the capture, processing, upload and sharing features that ShareX has and Snapline lacks. ShareX's unrelated utilities (DNS changer, hash checker, monitor test, IRC, social posting) are out of scope.

The rules in `progress-tauri.md` section 0 still apply: no code comments, UK English in the UI, no new dependency without approval, secrets never in a file, and Win32 only in `snapline-win` and `snapline-overlay`.

What changed in this revision:

- The first milestone is narrowed to one slice: pick a file, stream it to one destination, see progress, find it in History, get a link.
- The workflow redesign moves after the intake milestones, so it is designed against real constraints.
- The History migration follows SQLite's table-rebuild procedure; the first draft would have deleted every upload row.
- File ownership is explicit, so deleting a history row can never recycle a user's original.
- Paths never cross from the webview into the service.
- The whole deletion link is treated as a secret, not only its token.
- `.sxcu` `DestinationType` handling is stated as Snapline policy, and `{input}` templating moves forward.

## 1. Where the code stands today

Read from the repository on 2026-11-10. "Wired" means reachable by a user; "backend only" means the code exists but nothing reaches it.

| Area | State | Where |
| --- | --- | --- |
| Region, window, screen, last-region capture | Wired | `core/src/workflow/step.rs`, `services/src/capture.rs` |
| Video, GIF and window recording (FFmpeg `ddagrab`, audio through named pipes) | Wired | `core/src/ffmpeg_args.rs`, `services/src/recording.rs` |
| Annotation: rect, ellipse, arrow, pen, highlight, text, step, blur, crop | Wired | `core/src/annotate/shape.rs`, `overlay-windows/src/canvas_d2d.rs` |
| Workflow engine with 12 step types | Wired | `core/src/workflow/{step,run,mod}.rs` |
| Uploaders: Imgur, S3, SFTP, Gist, custom `.sxcu` | Wired | `crates/upload/src` |
| Upload from clipboard | Wired, images only, one file | `services/src/clipboard_upload.rs` |
| Open a history item or file in the editor; pin to screen | Wired from History | `services/src/editor.rs`, `HistoryPage.svelte` |
| Re-upload a history item elsewhere | Wired | `UploadReq::UploadElsewhere` |
| Per-type destinations | Five settings; only `images` and `videos_and_gifs` are read | `store/src/settings/uploads.rs`, `services/src/workflow_runner.rs:424` |
| Link shortener | Setting only | `store/src/settings/uploads.rs` |
| Video converter, image combiner, QR | Backend only since the Tools pages were removed | `services/src/tools/` |

Facts that constrain the design:

1. **A workflow must start with Capture or Record** (`Workflow::validate`). An existing file cannot run through one.
2. **`RunContext` holds parallel options** (`image`, `recording`, `converted`, `saved_file`) that can disagree about what the current input is (`core/src/workflow/run.rs:52`).
3. **`UploadFile` holds the whole file in memory** (`bytes: Vec<u8>`).
4. **Foreign keys are on before migrations run** (`store/src/history/migrate.rs:85`), all migrations share one transaction, and `upload.capture_id` is `ON DELETE CASCADE`.
5. **Deleting a history row recycles `capture.path`** (`services/src/history.rs:222`).
6. **Only queued or backing-off uploads can be cancelled** (`upload/src/queue.rs:288`). There is no in-flight cancel and no byte progress.
7. **The deletion link is not protected.** Imgur builds it with the delete hash in the URL (`upload/src/imgur.rs:185`), it is stored in plain text in `upload.deletion_url`, it is sent to the webview in `UploadDto.deletion_url` (`services/src/dto/history.rs:39`), and `DetailPane.svelte:517` has a button that copies it.
8. **Custom uploader templates know four placeholders:** `{filename}`, `{random}`, `{response}` and `{json:path}` (`upload/src/custom/template.rs:126`). There is no `{input}`. `DestinationType` is written on export and ignored on import.
9. **The webview can set any settings key** through `Services::update_settings`, which merges an arbitrary patch. `capture.ffmpeg_path` is a free-text field on `CapturePage.svelte:394`, so the webview already controls one executable path.
10. **History retention only expires upload rows** (`HistoryRepo::expire_uploads`). It never removes files.

## 2. Design rules for everything below

### 2.1 Ownership

Every history row says whether Snapline owns the file.

- `capture.owned = 1`: Snapline created the file (captures, recordings, clipboard images, clipboard text, converted and trimmed output).
- `capture.owned = 0`: the row borrows a path the user gave. Snapline never writes to it, moves it or recycles it.

| Operation | Owned | Borrowed |
| --- | --- | --- |
| Delete from History | Recycle the file, remove the row and thumbnail | Remove the row and thumbnail only. The confirmation says "The original file stays where it is." |
| Replace (trim, convert, external program with replace, effects) | Overwrite in place | Never. Output is written to the Snapline folder as a new owned row |
| Any future file retention | Applies | Never applies |
| File missing | Row shows as missing | Row shows as missing; re-upload is disabled |

`HistoryService::delete` branches on `owned` before it calls `recycle_file`. That branch gets a test that fails if a borrowed path is ever passed to the host.

### 2.2 A borrowed file that changes

A borrowed file can change between being queued and being sent, or before a retry. Snapline does not stage a copy, because that doubles disk use for large videos. It fails the job instead.

- At intake, record the size (`capture.bytes`, already there) and the modified time (`capture.source_modified_ms`, new).
- Each attempt opens the file once and reads metadata **from the open handle**. If the size or modified time differs, the attempt ends with a permanent error: "The file changed after it was added. Add it again to upload the new version."
- The reader is limited to the recorded length and reports an error if the file ends early, so a file truncated mid-read fails instead of uploading a short body.
- Retry uses the same check, so a retry sends the same bytes or nothing.

### 2.3 Trust boundary

The webview is the less trusted side. A file check and a size cap do not stop a compromised webview from asking Snapline to upload a private document, so no request from the webview carries a path.

- **Picker.** `UploadReq::PickAndUpload` has no path field. The service calls a native `FilePicker` port and uses what it returns.
- **Drag-and-drop.** The drop is handled in Rust, in the window event handler in `apps/snapline/src/windows.rs` (Tauri's `WindowEvent::DragDrop`). The webview only receives "hovering" and "dropped N files" for visuals.
- **When the UI must confirm first** (for example to choose a destination), the native side creates a selection and returns an opaque `SelectionId`: random, single use, expiring after five minutes, bound to the window that received the drop. The UI gets display names and sizes and sends back `UploadReq::UploadSelection { selection, dest }`.
- **Command line and Explorer.** Arguments arrive natively, with the trust of whoever launched the process.
- The size cap stays as a resource guard. It is not a security control.

The same principle covers settings that lead to execution or file access. `Services::update_settings` gets a denylist of keys the webview may not patch (`programs`, `watch_folders`, and `capture.ffmpeg_path`, which is an existing hole of the same kind). Those are changed through dedicated requests that take their paths from native dialogs.

### 2.4 Deletion links are secrets

The deletion link is a capability, whether or not it has a separate token.

- The whole `DeletionLink` (URL and token) is sealed before it is stored and never appears in a DTO, a log or an error.
- `UploadDto` and `UploadStateDto` lose `deletion_url` and `deletion_link` and gain `can_delete_remote: bool`.
- The "copy deletion link" button in `DetailPane.svelte` becomes a request (`HistoryReq::CopyDeletionLink { id }`) that copies natively after a native confirmation, or is removed. Decision 4.
- Storage and the migration of existing plain-text rows are in section 7.4.

### 2.5 `.sxcu` compatibility policy

ShareX documents `DestinationType` as metadata used when a `.sxcu` file is imported, to select the uploader for that destination type. Its example uses several comma-separated values. It is not a restriction on what the uploader accepts.

Snapline's policy:

- Parse `DestinationType` as a set: split on commas, trim, ignore case, accept `None`, `ImageUploader`, `TextUploader`, `FileUploader`, `URLShortener`, `URLSharingService`. Unknown values produce an `SxcuWarning`, not an error.
- On import, the set **pre-fills** the destination's "Use for" checkboxes (images, video, text, other files, link shortener). Those checkboxes are Snapline settings the user can change.
- Routing and filtering use the Snapline checkboxes, never the `.sxcu` field directly. An absent or `None` value pre-fills everything.
- Export writes the checkboxes back as a comma-separated `DestinationType`.
- Tests use real configs exported from ShareX, kept in `crates/upload/tests/fixtures/sxcu/`. Aaron supplies these (decision 6).

## 3. Milestone 1: upload a file and get a link

The first release. One path through the system, end to end:

native file picker → one streaming destination (SFTP) → progress and cancel → History → link on the clipboard.

It does not touch the workflow engine. The intake service queues uploads directly, as clipboard upload does today.

### 1.1 History migration `0004_files.sql`

SQLite cannot alter a `CHECK` constraint, so `capture` is rebuilt. With foreign keys on, `DROP TABLE capture` performs an implicit delete that cascades to `upload`. `PRAGMA foreign_keys` is also ignored inside a transaction, so the SQL file cannot fix this on its own. The migrator changes.

Migrator (`store/src/history/migrate.rs`):

1. `open_write` opens the connection and sets WAL as it does now.
2. If the schema is behind, take a backup first: `VACUUM INTO '<history>.bak'`. This is WAL-aware and needs no extra rusqlite feature. An older `.bak` is removed first; a failed backup aborts the upgrade.
3. `PRAGMA foreign_keys = OFF`, then read it back and abort if it is still on.
4. `BEGIN IMMEDIATE`, run the pending migrations, set `user_version`.
5. `PRAGMA foreign_key_check`. Any row returned rolls back and fails with a new `HistoryError::ForeignKeys`.
6. `COMMIT`, then `PRAGMA foreign_keys = ON` and read it back. A guard restores it on the error path too.

Migration:

```sql
CREATE TABLE capture_new (
  id                  INTEGER PRIMARY KEY,
  kind                TEXT    NOT NULL CHECK (kind IN ('image','video','gif','text','file')),
  origin              TEXT    NOT NULL DEFAULT 'capture' CHECK (origin IN ('capture','clipboard','file','url','watch')),
  owned               INTEGER NOT NULL DEFAULT 1 CHECK (owned IN (0, 1)),
  path                TEXT    NOT NULL,
  bytes               INTEGER NOT NULL,
  source_modified_ms  INTEGER,
  width               INTEGER,
  height              INTEGER,
  duration_ms         INTEGER,
  source_app          TEXT,
  created_at          INTEGER NOT NULL,
  ocr_text            TEXT
);
INSERT INTO capture_new (id, kind, path, bytes, width, height, duration_ms, source_app, created_at, ocr_text)
  SELECT id, kind, path, bytes, width, height, duration_ms, source_app, created_at, ocr_text FROM capture;
DROP TABLE capture;
ALTER TABLE capture_new RENAME TO capture;
CREATE INDEX capture_created ON capture(created_at DESC, id DESC);
```

Notes:

- `upload` still names `capture` in its foreign key, and resolves to the rebuilt table after the rename. `upload_capture` and `capture_sequence` are untouched.
- `capture_fts` is contentless and keyed by rowid. Ids are preserved, so it keeps working.
- `CaptureKind::parse` in the store gains `Text` and `File`.

Tests (all in `store`, no window, no network):

- Upgrade a version 3 database holding captures with zero, one and three uploads. Assert the upload count is unchanged, every upload still joins to its capture, full-text search still finds a known name, `capture_created` exists, `foreign_key_check` is empty and `foreign_keys` reads 1 afterwards.
- A migration that fails halfway leaves `user_version` at 3 and every row intact.
- The backup opens as a valid database with the pre-upgrade row counts.
- Existing rows come out `owned = 1`, `origin = 'capture'`.

### 1.2 Intake service

New `services/src/intake.rs`. It is the single entry point for content that did not come from a capture; later milestones add sources to it without adding paths through the system.

```rust
pub enum IntakeItem {
    File(PathBuf),
}

pub struct IntakeRequest {
    pub items: Vec<IntakeItem>,
    pub origin: Origin,
    pub dest: Option<String>,
}

pub struct Intake { files: Arc<dyn FilePicker>, history: ..., queue: Arc<dyn UploadQueue>, clipboard: ..., notifier: ... }

impl Intake {
    pub fn pick_and_upload(&self) -> Result<Vec<Queued>, ServiceError>;
    pub fn upload(&self, request: IntakeRequest) -> Result<Vec<Queued>, ServiceError>;
}
```

- `FilePicker` is a port in services; `platform-windows/src/dialogs.rs` gains `pick_files` (multi-select `IFileOpenDialog`) beside `pick_folder`.
- For each file: check it is a regular file below the size cap, classify it, insert a borrowed history row with its size and modified time, and enqueue.
- Classification lives in a new pure module `core/src/media.rs`: `classify(file_name, first_bytes) -> MediaKind` (extension table, then magic numbers) and `content_type(file_name)`. The second replaces the private `content_type` functions in `apps/snapline/src/upload_files.rs` and `services/src/workflow_runner.rs:519`.
- Destination: `destination_for(&settings.uploads, media)` in `services/src/uploads.rs`, mapping the media kind to one of the five routing settings. `QueueUploadHandOff` switches to it, replacing the inline choice at `workflow_runner.rs:424`. If the routed destination is "save locally only", the picker flow shows a native message naming the setting to change.
- New action `upload_file` in `core/src/actions.rs` (group `uploads`, no default chord), dispatched from `services/src/uploads.rs::run`, and a tray item. `UploadReq::PickAndUpload` exposes it to the UI with no path field.

### 1.3 Streaming body

```rust
pub enum UploadBody {
    Bytes(Vec<u8>),
    Path { path: PathBuf, modified_ms: i64 },
}

pub struct UploadFile {
    pub path: PathBuf,
    pub file_name: String,
    pub content_type: String,
    pub len: u64,
    pub body: UploadBody,
}

impl UploadFile {
    pub fn open(&self) -> Result<Box<dyn Read + Send>, UploadError>;
    pub fn read_all(&self, limit: u64) -> Result<Vec<u8>, UploadError>;
}

pub trait UploadControl: Send + Sync {
    fn sent(&self, bytes: u64);
    fn cancelled(&self) -> bool;
}

pub trait Uploader: Send + Sync {
    fn upload(&self, file: &UploadFile, secrets: &dyn SecretStore, control: &dyn UploadControl) -> Result<Uploaded, UploadError>;
    ...
}
```

- `open` performs the changed-file check from 2.2 and returns a length-limited reader.
- **Only SFTP streams in this milestone.** Imgur, S3, Gist and custom call `read_all` with a cap (64 MiB) and fail with a clear permanent error above it. This keeps the change to those uploaders mechanical and lets streaming and cancellation be proven on one destination first.
- `HistoryFileSource` (`apps/snapline/src/upload_files.rs`) stops reading the file and builds an `UploadBody::Path`.

SFTP (`upload/src/sftp.rs`):

- `SftpConnection::write_file` takes `source: &mut dyn Read`, `len` and `control` in place of `bytes: &[u8]`.
- It writes to `<name>.part` in 32 KiB blocks, calls `control.sent` after each block and checks `control.cancelled` before each. On success it renames to the final name, so the public URL never serves a partial file. On cancel or error it removes the `.part` file.
- The fake `SftpConnection` used in tests records the bytes it received, which is what the retry test asserts on.

### 1.4 Progress and in-flight cancel

- `UploadSnapshot` gains `sent: u64` and `total: u64`. The queue's `UploadControl` implementation updates them and emits an event at most every 100 ms or 1 percent.
- Each running job has an `Arc<AtomicBool>`. `Handle::cancel` on an `Uploading` job sets it; the uploader returns a new non-retryable `UploadError::Cancelled`; the job ends `Cancelled`. `QueueError` loses the "only queued or backing-off" restriction.
- `UploadDto` and `UploadStateDto` gain `sent` and `total`. `DetailPane.svelte`, the History rows and `QuickPanel.svelte` show a bar and a cancel button while uploading.

### 1.5 Link back

Upload completion is handled at `services/src/uploads.rs:386`, which records the last link. Jobs queued by the intake service carry `origin`; on completion the handler also copies the link with `Clipboard::set_text` in the default link format and shows the existing "link copied" toast. Several files queued together copy their links as one newline-separated block when the last one finishes.

### 1.6 Acceptance

Done when all of these hold:

- Choosing a 1 GiB file uploads it over SFTP with resident memory staying flat (measured in a test with a fake connection and a generated file; the bound is the block size plus overhead, not the file size).
- Progress events arrive in order and end at `sent == total`.
- Cancelling mid-transfer ends the job as cancelled, removes the remote `.part` file, and leaves the history row and the local file alone.
- **Retry test.** A job fails on the first attempt and succeeds on the second: the bytes received on the second attempt equal the original file. A second case modifies the file between attempts: nothing is sent, and the job fails with the "file changed" message.
- **Ownership test.** Deleting the history row for an uploaded borrowed file leaves the file in place and never calls `recycle_file`. Deleting an owned capture still recycles it.
- The migration tests in 1.1 pass.
- The row appears in History with kind `file` (or image or video), can be filtered, and "Upload elsewhere" works on it.
- No request type reachable from the webview contains a file path.

## 4. Milestone 2: clipboard text and several files

Same intake service, new item types.

- `IntakeItem` gains `Image(Image)` and `Text(String)`. `core::ports::Clipboard` gains `get_text` and `get_files`; `platform-windows/src/clipboard.rs` implements them with `CF_UNICODETEXT` and `CF_HDROP` (`DragQueryFileW`).
- Priority when several formats are present: files, then image, then text.
- Clipboard images and text have no file, so Snapline saves them (owned): images as today, text to `<save folder>/Text/<name>.txt` through the naming pattern.
- Several files: one upload each. `ClipboardUploadError::MultipleFiles` is replaced by a confirmation naming the count.
- `services/src/clipboard_upload.rs` shrinks to the confirmation prompt plus a call to `Intake::upload`.

Destination capabilities, so a destination can refuse what it cannot host before the queue tries:

```rust
pub struct Accepts { pub images: bool, pub video: bool, pub text: bool, pub other: bool }
```

- Built in: Imgur accepts images and video; Gist accepts text; S3 and SFTP accept everything.
- Custom: the "Use for" checkboxes from 2.5. `DestinationViewDto` gains `accepts`, and `DestinationSelectRow.svelte` filters each of the five routing rows.

`{input}` templating, needed now for text uploaders:

- `TemplateContext` gains `input: Option<&str>`. `{input}` is valid in arguments, the URL and the body data.
- A custom text upload whose request uses `{input}` sends the text as that value and attaches no file. One that does not use it sends the text as a file under `FileFormName`. Confirm the second behaviour against ShareX with a real config before relying on it.
- The remaining ShareX placeholders are a backlog, in this order: `{header:name}`, `{responseurl}`, `{regex:...}`, `{base64:...}` (no dependency needed); `{xml:xpath}` (needs a parser; ask first); `{select}`, `{inputbox}` and `{outputbox}` are interactive and unsupported, and importing a config that uses them raises an `SxcuWarning`.

Done when copying text and pressing Ctrl+Shift+V creates a Gist; copying three files in Explorer and pressing it uploads three; and a real ShareX text-uploader config round-trips through import and export.

## 5. Milestone 3: drag-and-drop, command line, Explorer

All three call `Intake::upload`.

- **Drag-and-drop:** handled natively as in 2.3. `MainShell.svelte` draws the drop target from hover events.
- **Command line:** `--upload <path>...` in `apps/snapline/src/cli.rs`, consuming the remaining arguments. `Args::parse_forwarded` already handles second launches.
- **First launch:** a primary instance ignores its own `--action`. After setup completes, run the same handler the single-instance callback uses. Without this, "Upload with Snapline" does nothing when Snapline is not running.
- **Explorer verb:** new `platform-windows/src/shell_menu.rs`, modelled on `autostart.rs`. It writes `HKCU\Software\Classes\*\shell\Snapline` with `MUIVerb`, `Icon`, `MultiSelectModel = Player` and `command = "<exe>" --upload "%1"`. Toggle on the General page; removal in `services/src/uninstall.rs`.
- On Windows 11 the verb appears under "Show more options". A top-level entry needs a packaged `IExplorerCommand` handler; deferred.

Done when a file dropped on the window, a file passed on the command line with Snapline closed, and a file sent from Explorer each end with a link on the clipboard.

## 6. Milestone 4: workflow redesign

Designed after milestones 1 to 3, using what they showed about typing, ownership and progress.

### 4.1 One tagged input, outputs tracked apart

```rust
pub enum Artefact {
    Image(Image),
    File { path: PathBuf, media: MediaKind, owned: bool },
    Text(String),
}

pub enum OutputRole { Saved, Converted, Thumbnail }

pub struct Output { pub role: OutputRole, pub path: PathBuf, pub capture_id: Option<i64> }

pub struct RunContext {
    pub current: Option<Artefact>,
    pub outputs: Vec<Output>,
    pub extracted_text: Option<String>,
    pub link: Option<String>,
    ...
}
```

- `current` is the one thing the next step acts on. A step that transforms it replaces it: Annotate and Effects map image to image; Convert to GIF maps a video file to a GIF file and records an `Output`.
- `image`, `recording`, `converted` and `saved_file` are removed, so they cannot disagree. `SaveRequest` and `UploadRequest` take `&Artefact`.
- A borrowed `File` artefact is never written to. Save on a borrowed file is a no-op that reports the existing path.
- `File { media: Image }` is decoded on demand, through the codec port, when a pixel step needs it.

### 4.2 Import step and dynamic types

```rust
pub enum ImportSource { Clipboard, File, Url }
Step::Import { source: ImportSource }
```

A source's type may only be known at run time, so checking has two levels.

| Source | May produce |
| --- | --- |
| Capture | image |
| Record video or window | video file |
| Record GIF | GIF file |
| Import clipboard | image, any file, text |
| Import file or URL | any file |

- Each step declares the media it accepts. **At edit time**, `Workflow::validate` rejects a step whose accepted set has no overlap with what can reach it (Annotate after Record, Convert to GIF after Capture).
- **At run time**, before a step that needs pixels, the runner checks the actual artefact. A mismatch on a partly compatible workflow (Annotate after Import clipboard, when the clipboard held a zip) records `StepStatus::Skipped { reason }` and continues. The report lists skipped steps and the toast mentions them.
- `WorkflowRunner::run_with_input(workflow_id, Artefact)` lets the intake service run a user's workflow on imported content. The `Import` step is then already satisfied.
- Routing: each of the five destination settings can name a workflow to run for intake of that type, defaulting to the direct upload from milestone 1.

## 7. Milestone 5 onward: tracks with their own acceptance

Each track below is independent after milestone 4 unless stated, and each has its own done-when.

### 7.1 Streaming for the other destinations

- **S3:** a single `PUT` signed with `UNSIGNED-PAYLOAD` up to 64 MiB; multipart upload above it, each part hashed and retried on its own, with `AbortMultipartUpload` on cancel. Done when the retry and cancel tests from milestone 1 pass against a `tiny_http` fake, and a cancelled multipart leaves no parts.
- **Imgur, Gist, custom:** `http::Request::body` and `Multipart::file` accept a reader plus a length. Done when the 64 MiB cap is gone for custom uploaders.

### 7.2 Upload from URL

`Import { source: Url }` downloads with the existing `HttpClient` to a temporary owned file: http and https only, a redirect limit, a size cap enforced while reading. Done when a URL on the clipboard can be rehosted and an oversized response is abandoned without filling the disk.

### 7.3 After-upload actions

- `resume_after_upload` replays every step after `Upload` that needs a link, not only `CopyLink`.
- **Shortener:** `trait Shortener { fn shorten(&self, url: &str) -> Result<String, UploadError>; }`, implemented first for custom uploaders marked as link shorteners, using `{input}`. `Step::ShortenLink` keeps the original as `direct_link` and stores `upload.short_url` (a plain `ALTER TABLE ADD COLUMN`). Failure keeps the long link and warns. Resume must run off the UI thread, since this is network I/O.
- **Open link** (`shell::open_url`, already http and https only) and **Show QR** (`core::qr::make_image` into the pin window).

### 7.4 Remote deletion

Storage: the sealed `DeletionLink` goes in a new `upload.deletion_sealed BLOB` column, encrypted with DPAPI for the current user through a `Protector { seal, open }` port implemented in `snapline-win`. Rows expire with `expire_uploads`, so nothing is left behind. The alternative is one Credential Manager entry per upload, which keeps rule 8 to the letter but grows without bound and needs separate clean-up. Decision 4.

Migrating existing rows:

1. Set `PRAGMA secure_delete = ON`.
2. For each row with a plain `deletion_url`: seal it, write `deletion_sealed`, null the old column.
3. Checkpoint and truncate the WAL so the plain text does not survive in it.
4. Delete `history.bak` from milestone 1 if it predates this step, because it holds the plain values.

This runs in services on first start after the upgrade, since the store crate cannot call DPAPI. A later migration drops the old column.

Behaviour:

- `HistoryReq::DeleteRemote { id }` opens the sealed link, builds the uploader for `upload.dest_id` and calls `Uploader::delete`. Imgur has an implementation; S3 is a `DELETE`; SFTP is `unlink`; custom uses the `.sxcu` `DeletionURL`.
- UI: "Delete from host" in the detail pane, and "Also delete the uploaded copy" in the local delete confirmation.

Done when no DTO, log line or SQLite page contains a deletion URL after migration (tested by scanning the database file for a known hash), and deleting from Imgur works end to end against a fake.

### 7.5 More workflow steps

| Step | Accepts | Implementation |
| --- | --- | --- |
| `CopyFile` | saved file | `Clipboard::set_files` (`CF_HDROP`) |
| `CopyPath` | saved file | `Clipboard::set_text` |
| `ShowInFolder`, `OpenFile` | saved file | `shell::show_in_folder`, `shell::open_file` |
| `SaveThumbnail { max_width, suffix }` | image | `thumbs::write_thumbnail` beside the saved file |
| `SaveAs` | image or file | Native save dialog; cancel reports `Cancelled` |
| `Pin` | image | `overlay-windows/src/pin.rs` |
| `ScanQr` | image | `core::qr::scan` into `extracted_text` |
| `Print` | saved file | `ShellExecuteW` with the `print` verb |
| `Delay { ms }` | anything | Capped at 10 s |

Shared edits for each: `Step` and its label, `StepDto`, the accepted-media table from 4.2, and `StepEditor.svelte`. The step picker groups into Source, Edit, Output, Share.

### 7.6 Image effects

New pure module `core/src/effects/` over `Image`.

| Effect | Parameters | Notes |
| --- | --- | --- |
| Resize | mode, filter | Move `saver::scale_to_width` here. Area average down, bilinear up. |
| Canvas | padding per side, colour or transparent | |
| Border | width, colour, inside or outside | Reuses the rect stroke in `annotate/raster.rs`. |
| Rounded corners | radius | Anti-aliased alpha mask. |
| Shadow | offset, blur radius, colour, opacity | Three box-blur passes over alpha. |
| Watermark image | file, anchor, offset, opacity, scale | Decoded through `ImageCodec`. |
| Watermark text | text with naming tokens, font, size, colour, anchor | Needs the text port below. |
| Background | colour or gradient behind a padded capture | |

- Text: `annotate/raster.rs` has only a 5x7 bitmap font. Add a `TextRasteriser` port implemented in `snapline-win` with DirectWrite. It also improves final-render text for annotations.
- Presets are ordered lists stored as `[[effects.presets]]` in a new `store/src/settings/effects.rs`. `Step::Effects { preset }` runs through an `EffectsPort`.
- If an effect adds transparency and the save format is JPEG, the saver flattens onto a configured colour.
- A watermark image path is chosen through a native dialog (2.3).
- UI: an Effects settings page with a preview rendered by the backend and served on the custom protocol, as `protocol_combine.rs` does for combine.

Done when each effect has a snapshot test on a synthetic image and a preset applied in a workflow produces the same pixels as the preview.

### 7.7 Run external program

```rust
pub struct Program { pub id: ProgramId, pub name: String, pub path: PathBuf, pub args: String, pub extensions: Vec<String>, pub timeout_s: u32, pub replace_output: bool }
Step::RunProgram { program: ProgramId }
```

Execution: `args` is a template with `{input}` and `{output}`, tokenised by Snapline and passed as an argument vector with no shell. The process runs with `CREATE_NO_WINDOW` inside a job object with kill-on-close, and is killed at the timeout. It never runs on a borrowed file in place; the input is copied to a temporary owned file first.

Security. The executable, its arguments and the set of workflows that may invoke it are all sensitive:

- `programs` is on the `update_settings` denylist. Changes go through `ProgramReq::{Add, Edit, Remove}`.
- The path comes only from a native file dialog.
- Any add, any change to the arguments, timeout or replace flag, and any workflow save that adds a `RunProgram` step shows a native confirmation (`platform-windows/src/task_dialog.rs`) with the full command line and the names of the workflows that will run it. Declining leaves the settings unchanged.

Done when a test proves a settings patch cannot create or alter a program, and a file name containing quotes and ampersands reaches the child as one argument.

### 7.8 Editor tools

Each new shape touches the same places: the `Shape` variant with bounds and hit testing (`core/src/annotate/shape.rs`), software rendering (`raster.rs`), live rendering (`overlay-windows/src/canvas_d2d.rs`), the `Tool` variant (`core/src/overlay/state.rs`), the toolbar (`icons.rs`, `toolbar.rs`), the shortcut hint (`hints.rs`) and a design icon.

| Shape | Model | Rendering |
| --- | --- | --- |
| Line | two points, style | Existing `line` |
| Pixelate | rect, block size | `raster.rs` already has `pixelate`; today's Blur uses it. Split into `Blur { radius }` and `Pixelate { block }`; old documents load `blur` as pixelate |
| Spotlight | rect, dim opacity | Fill outside the rect; drawn first |
| Magnify | source rect, target rect, zoom | Scaled copy of the frozen image |
| Image or sticker | embedded bitmap, rect | `draw_image` exists |
| Speech bubble | rect, tail point, text | Rounded rect, triangle, text |
| Redact | rect, colour | Opaque fill |
| Eraser | a tool, not a shape | Emits delete commands through `annotate/command.rs`, so undo works |

Also: text font size, background and outline; rotate, flip, resize and add canvas from `core/src/effects`; and "Combine" on a multi-selection in History using the existing `ToolReq::CombineSave`. Saving an edited borrowed image creates a new owned row.

### 7.9 History management

- **Filters.** `HistoryFilter` becomes a struct: kinds, upload state, destination, source app, date range, origin. `repo.rs` builds the `WHERE` clause from fixed fragments with bound parameters. Indexes on `capture(source_app)` and `upload(dest_id)`.
- **Multi-select and bulk actions.** `HistoryReq::Bulk { ids, action }` with Delete, Upload to, Copy links, Combine; runs on the worker and reports partial failure through `HistoryAreaEvent`. Bulk delete applies the ownership rule per row and the confirmation states how many originals are kept.
- **All uploads per capture.** `HistoryCaptureDto.upload` becomes a list.
- **Export** as JSON without sealed links; optional import of ShareX's `History.json`.

### 7.10 Recording

- **Pause and resume by segments.** `ddagrab` cannot pause. `RecordingState` gains `Paused`; pause stops the current FFmpeg process and keeps the segment; resume starts `part-N`; stop joins them with the concat demuxer and stream copy (`concat_command` in `ffmpeg_args.rs`). Audio threads pause too. Measure resume latency; above about 300 ms, show the countdown.
- **Trim.** `trim_command { input, start_ms, end_ms, precise }`: stream copy on keyframes, or re-encode. Reuse the convert job plumbing as a general `MediaJob`. UI in the detail pane with a `<video>` on the custom protocol. Replace is offered only for owned files.
- **Converter re-entry** from the detail pane using `ToolReq::ConvertStart`.
- **Per-workflow options:** `Step::Record { kind, options: Option<RecordOptions> }`, falling back to global settings field by field.

### 7.11 More destinations

- **FTP and FTPS,** mirroring `sftp.rs` with a connector trait for tests, streaming from the start, and the `.part` then rename pattern. Needs a dependency decision (`suppaftp` with rustls).
- **OAuth infrastructure** in `upload/src/oauth.rs`: authorisation code with PKCE and a loopback redirect (`tiny_http` is already a dependency of the upload crate), state check, single-use listener, five-minute timeout. The refresh token goes in Credential Manager; the access token stays in memory. A `TokenProvider` is injected into uploaders and refreshes on expiry and once on a 401.

| Provider | Upload | Share link | Scope |
| --- | --- | --- | --- |
| Dropbox | `files/upload` to 150 MB, upload sessions above | `sharing/create_shared_link_with_settings`, rewritten to `raw=1` | `files.content.write`, `sharing.write` |
| Google Drive | Resumable upload | `permissions.create` for anyone, then `webViewLink` | `drive.file` |
| OneDrive | `PUT` for small files, `createUploadSession` above | `createLink`, view, anonymous | `Files.ReadWrite` |

Each provider is accepted on its own: connect, upload a file above the chunk threshold with one chunk failing once, cancel mid-upload, disconnect.

### 7.12 Automation

- **Auto capture.** A timer enqueues a capture of a fixed region, a monitor or the last region and runs a chosen workflow. Minimum interval 1 s; a tick is skipped if the previous one is still running; optional skip-if-unchanged by frame hash; stops on lock or sleep; the tray shows it is running.
- **Watch folder.** A `FolderWatcher` port implemented with `ReadDirectoryChangesW`. `watch_folders` is on the settings denylist and its path comes from the native folder picker. A file is processed once its size is stable across two checks and it can be opened without write sharing. Snapline's own save folder and its parents are refused. Watched files are borrowed. A buffer overflow triggers a rescan compared against History by path.

## 8. Order

Sizes are relative: S is a day or two of focused work, M about a week, L more.

| Order | Work | Size | Depends on |
| --- | --- | --- | --- |
| 1 | Milestone 1: file to link | L | none |
| 2 | Milestone 2: clipboard text, several files, capabilities, `{input}` | M | 1 |
| 3 | Milestone 3: drag-and-drop, command line, Explorer | M | 1 |
| 4 | Remote deletion (7.4) | M | 1 |
| 5 | Milestone 4: workflow redesign | L | 1 to 3 |
| 6 | Streaming for S3 and HTTP uploaders (7.1) | M | 1 |
| 7 | After-upload actions (7.3), more steps (7.5) | M | 5 |
| 8 | Effects (7.6), external programs (7.7) | L | 5 |
| 9 | Editor tools (7.8) | L | text port from 7.6 |
| 10 | History management (7.9) | M | 1 |
| 11 | Recording (7.10) | M | none |
| 12 | FTP, OAuth providers (7.11) | L | 6 |
| 13 | URL intake (7.2), automation (7.12) | M | 5 |

Remote deletion is placed early because it closes an existing exposure, not because it is a parity feature. Recording touches files nothing else does and can run alongside the main line.

## 9. Decisions needed from Aaron

1. **Migration backup:** keep `history.bak` until the deletion-link migration removes it, or delete it after the next successful start?
2. **Per-destination cap for non-streaming uploaders** in milestone 1: 64 MiB proposed.
3. **Run-time type mismatch in a workflow:** skip the step and report it (proposed), or fail the run?
4. **Deletion links:** DPAPI-sealed column (proposed) or Credential Manager entries. And should "copy deletion link" stay, behind a native confirmation, or go?
5. **`capture.ffmpeg_path`:** move it behind a native file dialog now, as part of the settings denylist in milestone 1, or later?
6. **`.sxcu` fixtures:** can you export a few real configs (image, text, file, shortener) for the tests?
7. **FTP:** approve `suppaftp`, or hand-write a client?
8. **OAuth:** will you register apps with Dropbox, Google and Microsoft?
9. **Explorer menu on Windows 11:** accept "Show more options" for now?
10. **Design:** progress bar, drop target, "Use for" checkboxes, new toolbar icons and the Effects page need design input.
11. **Blur split:** rename today's Blur to Pixelate and add a true blur?

## 10. Risks

- **The migration is the riskiest change and it ships first.** It is covered by the upgrade, failure and backup tests in 1.1, and it runs once per user.
- **Changing the `Uploader` trait** touches all five uploaders and their fakes at once. Keeping four of them on `read_all` makes that change mechanical.
- **Borrowed paths go stale.** Files move and drives disconnect. History must show a missing file plainly and never treat it as an error to retry forever; the changed-file error is permanent for the same reason.
- **The settings denylist must fail closed.** A test enumerates every key under the protected sections and asserts a patch to each is rejected.
- **Memory.** Effects and the editor hold full-size BGRA images; a 4K capture is 33 MB per copy. Effects work in place where possible and drop intermediates, so the idle budget in `progress-tauri.md` section 6 holds after a workflow finishes.
