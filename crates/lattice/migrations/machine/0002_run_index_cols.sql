-- Lattice machine store, migration 0002.
-- Source of truth: FACET_HANDOFF_BRIEF.md, Surface 1 (Evan, 2026-09-06).
--
-- The cross-workspace run_index gains duration_ms and actor so the machine
-- index can answer "how long did this run take" and "who ran it" without
-- joining back to the workspace store. Existing pointer rows backfill to
-- NULL for both columns; new rows set them via index_run.

ALTER TABLE run_index ADD COLUMN duration_ms INTEGER;
ALTER TABLE run_index ADD COLUMN actor TEXT;
INSERT INTO schema_version VALUES (2, strftime('%s','now') * 1000);
