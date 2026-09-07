-- Lattice workspace store, migration 0003.
-- Source of truth: fullstack deep dive 2026-09-07 §5.2 (Goal 4, replay + diff).
--
-- replayed_from: runs.id of the run this one replayed, NULL otherwise.
-- var_names:     JSON array of the --var NAMES passed at resolve time. Names
--                only; values never persist anywhere in Lattice.
--
-- Reader rule for rows recorded before this migration:
--   replayed_from IS NULL -> not a replay
--   var_names IS NULL     -> unknown (pre-0003), distinct from '[]' = none
--
-- Git HEAD does not get a column (auto-tag later, if ever).

ALTER TABLE runs ADD COLUMN replayed_from TEXT;
ALTER TABLE runs ADD COLUMN var_names TEXT;
CREATE INDEX runs_replayed_from ON runs(replayed_from);
INSERT INTO schema_version VALUES (3, strftime('%s','now') * 1000);
