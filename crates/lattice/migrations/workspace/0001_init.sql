-- Lattice workspace store, migration 0001.
-- Times are Unix milliseconds UTC. IDs are ULIDs stored as TEXT.

CREATE TABLE schema_version (
  version     INTEGER PRIMARY KEY,
  applied_at  INTEGER NOT NULL
);
INSERT INTO schema_version VALUES (1, strftime('%s','now') * 1000);

-- One row per executed request.
CREATE TABLE runs (
  id              TEXT PRIMARY KEY,            -- ULID
  started_at      INTEGER NOT NULL,
  duration_ms     INTEGER,
  request_path    TEXT NOT NULL,               -- repository selector, e.g. users/list-users.yml
  request_hash    TEXT NOT NULL,               -- sha256 of the resolved request (after env substitution)
  environment     TEXT,                        -- environment name used
  method          TEXT NOT NULL,
  url             TEXT NOT NULL,               -- resolved URL, secrets redacted
  status          INTEGER,                     -- HTTP status; NULL on transport failure
  error           TEXT,                        -- transport/timeout error text
  req_headers     TEXT,                        -- JSON, secrets redacted
  res_headers     TEXT,                        -- JSON
  req_body_len    INTEGER,
  req_body        BLOB,                        -- inline if <= inline_body_max
  req_body_hash   TEXT,                        -- set iff body is a blob file
  res_body_len    INTEGER,
  res_body        BLOB,
  res_body_hash   TEXT,
  res_content_type TEXT,
  session_id      TEXT,                        -- machine-store sessions.id; NULL for human runs
  actor           TEXT NOT NULL DEFAULT 'human', -- 'human' | agent name
  tags            TEXT                         -- JSON array
);
CREATE INDEX runs_started_at   ON runs(started_at DESC);
CREATE INDEX runs_request_path ON runs(request_path, started_at DESC);
CREATE INDEX runs_status       ON runs(status);
CREATE INDEX runs_res_hash     ON runs(res_body_hash);

-- Blob registry. One row per distinct file in .facet/blobs/.
CREATE TABLE blobs (
  hash          TEXT PRIMARY KEY,              -- sha256 hex
  len           INTEGER NOT NULL,
  content_type  TEXT,
  created_at    INTEGER NOT NULL
);

-- Reader rule (enforced in code, documented here):
--   if res_body_hash IS NOT NULL -> read .facet/blobs/<hash>
--   else                          -> read res_body
-- Never branch on res_body_len.
