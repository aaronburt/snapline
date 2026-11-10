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
ALTER TABLE upload ADD COLUMN sent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE upload ADD COLUMN total INTEGER NOT NULL DEFAULT 0;
