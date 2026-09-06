-- Lattice machine store, migration 0001.
-- Source of truth: FACET_HANDOFF_BRIEF.md, Section III (Surface 1 default).

CREATE TABLE schema_version (
  version     INTEGER PRIMARY KEY,
  applied_at  INTEGER NOT NULL
);
INSERT INTO schema_version VALUES (1, strftime('%s','now') * 1000);

CREATE TABLE workspaces (
  id          TEXT PRIMARY KEY,                -- ULID, written into .facet/workspace.toml on init
  path        TEXT NOT NULL,                   -- last known absolute path
  name        TEXT,
  last_seen   INTEGER NOT NULL
);

-- Cross-workspace run index. One pointer row per run.
CREATE TABLE run_index (
  run_id        TEXT PRIMARY KEY,
  workspace_id  TEXT NOT NULL,
  started_at    INTEGER NOT NULL,
  request_path  TEXT NOT NULL,
  status        INTEGER
);
CREATE INDEX run_index_started ON run_index(started_at DESC);

CREATE TABLE sessions (
  id          TEXT PRIMARY KEY,                -- ULID
  actor       TEXT NOT NULL,                   -- agent name
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  meta        TEXT                             -- JSON
);

CREATE TABLE environments (
  workspace_id  TEXT NOT NULL,
  name          TEXT NOT NULL,
  key           TEXT NOT NULL,
  value         TEXT,                          -- plain if not secret
  secret_ref    TEXT,                          -- keyring reference if secret (Surface 3); value NULL
  updated_at    INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, name, key)
);

CREATE TABLE preferences (
  key         TEXT PRIMARY KEY,
  value       TEXT NOT NULL                    -- JSON
);
