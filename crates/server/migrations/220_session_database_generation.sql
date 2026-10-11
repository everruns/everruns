-- Write generation for persisted session SQL databases.
--
-- Decisions:
-- - Each server process keeps a session database as an in-memory SQLite cache
--   and persists a write by upserting only the 4 KB pages that changed. Two
--   processes can hold a cache of the same database, so a write must prove its
--   cache is current before it replaces pages.
-- - `generation` is that proof: a write commits only when the row still has
--   the generation the writer's cache was loaded at (compare-and-set), and
--   bumps it in the same transaction as the page upserts. A writer that loses
--   the race reloads from the stored pages and retries instead of overwriting
--   another process's pages. Readers compare it to decide whether to reload.
-- - Existing rows start at 0; nothing persisted pages before this migration.

ALTER TABLE session_databases
    ADD COLUMN generation BIGINT NOT NULL DEFAULT 0;
