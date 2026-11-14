ALTER TABLE capture ADD COLUMN operation_id TEXT;
ALTER TABLE capture ADD COLUMN current_revision INTEGER NOT NULL DEFAULT 1;
CREATE UNIQUE INDEX capture_operation ON capture(operation_id) WHERE operation_id IS NOT NULL;
CREATE TABLE capture_revision (
  capture_id  INTEGER NOT NULL REFERENCES capture(id) ON DELETE CASCADE,
  revision    INTEGER NOT NULL CHECK (revision >= 1),
  path        TEXT    NOT NULL,
  bytes       INTEGER NOT NULL,
  width       INTEGER,
  height      INTEGER,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (capture_id, revision)
) WITHOUT ROWID;
INSERT INTO capture_revision (capture_id, revision, path, bytes, width, height, created_at)
  SELECT id, 1, path, bytes, width, height, created_at FROM capture;
