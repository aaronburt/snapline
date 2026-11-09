CREATE TABLE capture_sequence (last_id INTEGER NOT NULL);
INSERT INTO capture_sequence SELECT COALESCE(MAX(id), 0) FROM capture;
