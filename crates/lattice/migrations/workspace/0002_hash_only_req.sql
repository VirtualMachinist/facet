-- Lattice workspace store, migration 0002.
-- Source of truth: FACET_HANDOFF_BRIEF.md, Surface 1 (Evan, 2026-09-06).
--
-- Request bodies are hash-only. The inline `req_body` column is dropped;
-- request bodies always live in .facet/blobs/<req_body_hash>. A Rust
-- pre-migration step (see `hydrate_inline_request_bodies` in store.rs) runs
-- before this migration when upgrading from v1: it writes any inline
-- request bodies to blob files and sets req_body_hash, so no body is lost.
--
-- Response bodies are unchanged: res_body stays inline at or under the
-- 64KiB threshold, over it in a blob file keyed by res_body_hash.
--
-- Reader rule (unchanged): hash column set -> blob file; else inline column.
-- Never branch on length.

ALTER TABLE runs DROP COLUMN req_body;
INSERT INTO schema_version VALUES (2, strftime('%s','now') * 1000);
