-- Keep a dead task's activity options in its dead-letter row.
--
-- A requeue used to rebuild the task with default options, which dropped its
-- named queue (`ActivityOptions::queue`, migration 179) and retry settings, so
-- a requeued framework step could land on the default queue where nothing
-- claims it. The row now records the options; rows from before this change
-- stay NULL and requeue with defaults as before.
--
-- Nullable with no default, so adding it rewrites nothing. The crate's own
-- schema (crates/durable/schema/postgres.sql) carries the same change.

ALTER TABLE durable_dead_letter_queue ADD COLUMN options JSONB;
