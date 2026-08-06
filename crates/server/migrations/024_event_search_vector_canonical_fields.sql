-- Rebuild events.search_vector with canonical message text fields.
--
-- Migration 006 (v0.8.5) seeded a generated tsvector column from
-- `data->>'content'` only. PR #1566 ("fix(storage): search nested message
-- content in event filters") needed to also cover `data.message.content`,
-- `data.result`, `data.delta`, and `data.accumulated`, and was originally
-- delivered by editing migration 006 in place. That broke startup against
-- already-deployed databases because sqlx tracks each migration's checksum
-- in `_sqlx_migrations` and refuses to run when an applied migration's
-- content changes (see knowledge/operations/migrations.md, knowledge/project/release-process.md
-- "Migration Handling").
--
-- This migration restores the contract by:
--   1. Reverting migration 006 to its original v0.8.18 form (so the
--      checksum on disk matches the checksum recorded in databases that
--      already applied 006).
--   2. Dropping and re-creating `events.search_vector` and its supporting
--      partial GIN index here, with the canonical fields, as an additive
--      migration that all environments will apply cleanly.

-- Drop the existing partial GIN index, then the generated column it
-- referenced. Done explicitly (rather than relying on `DROP COLUMN`'s
-- automatic dependency cleanup) so the dependency is obvious in review.
DROP INDEX IF EXISTS idx_events_search_vector;
ALTER TABLE events DROP COLUMN search_vector;

-- Recreate the generated tsvector column from canonical message text
-- fields in the JSONB payload. Coalesce in priority order:
--   data.message.content   - chat-style messages (array stringified)
--   data.result            - tool/function results
--   data.content           - legacy / non-message events with top-level text
--   data.delta             - streaming deltas
--   data.accumulated       - accumulated streaming text
--
-- The canonical text is bounded with `LEFT(..., 250000)` because Postgres'
-- `to_tsvector` rejects inputs exceeding 1 MiB ("string is too long for
-- tsvector ... max 1048575 bytes") and individual rows can carry tool
-- results, accumulated streaming text, or stringified message-content
-- arrays well beyond that. 250 000 chars is a safe upper bound across
-- realistic UTF-8 inputs (250000 chars * 4 worst-case bytes/char =
-- 1000000 bytes, which stays below the 1048575-byte limit), and search
-- relevance is dominated by early tokens, so the truncation has
-- negligible effect on the index's usefulness.
ALTER TABLE events
    ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        to_tsvector(
            'english',
            LEFT(
                COALESCE(
                    data #>> '{message,content}',
                    data #>> '{result}',
                    data->>'content',
                    data->>'delta',
                    data->>'accumulated',
                    ''
                ),
                250000
            )
        )
    ) STORED;

-- Recreate the partial GIN index on the new column, scoped to the same
-- event types as the original.
CREATE INDEX idx_events_search_vector
    ON events USING GIN (search_vector)
    WHERE event_type IN (
        'input.message',
        'output.message.completed',
        'output.message.delta',
        'tool.completed'
    );

COMMENT ON COLUMN events.search_vector IS 'Generated tsvector for full-text search on canonical event text fields (EVE-87)';
