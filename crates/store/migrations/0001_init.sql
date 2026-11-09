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
