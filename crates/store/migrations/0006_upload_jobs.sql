CREATE TABLE upload_job (
  id                 INTEGER PRIMARY KEY,
  capture_id         INTEGER REFERENCES capture(id) ON DELETE SET NULL,
  revision           INTEGER,
  operation_id       TEXT,
  dest_id            TEXT    NOT NULL,
  config_fingerprint TEXT    NOT NULL,
  run_id             TEXT,
  batch_id           TEXT,
  batch_index        INTEGER,
  batch_size         INTEGER,
  file_name          TEXT    NOT NULL,
  content_type       TEXT    NOT NULL,
  file_path          TEXT    NOT NULL,
  file_len           INTEGER NOT NULL,
  file_modified_ms   INTEGER NOT NULL,
  staged             INTEGER NOT NULL DEFAULT 0 CHECK (staged IN (0, 1)),
  state              TEXT    NOT NULL CHECK (state IN (
    'waiting_history', 'queued', 'uploading', 'waiting_retry', 'paused',
    'needs_attention', 'outcome_unknown', 'complete', 'cancelled'
  )),
  attempts           INTEGER NOT NULL DEFAULT 0,
  next_retry_at      INTEGER,
  owner              TEXT,
  url                TEXT,
  page_url           TEXT,
  deletion_url       TEXT,
  error              TEXT,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL,
  finished_at        INTEGER
);
CREATE INDEX upload_job_state ON upload_job(state, id);
CREATE INDEX upload_job_capture ON upload_job(capture_id, id DESC);
CREATE INDEX upload_job_operation ON upload_job(operation_id) WHERE operation_id IS NOT NULL;
CREATE TABLE upload_queue_state (
  id     INTEGER PRIMARY KEY CHECK (id = 1),
  paused INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1))
);
INSERT INTO upload_queue_state (id, paused) VALUES (1, 0);
