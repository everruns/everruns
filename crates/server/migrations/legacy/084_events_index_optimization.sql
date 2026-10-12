-- Events index optimization for the hot read/write paths.
--
-- `events` is the highest-volume table (one row per streamed event) and is read
-- on every agent turn. Three issues hurt under load:
--
-- 1. Predicate mismatch made the per-turn message read unable to use its index.
--    `list_message_events` (storage/repositories/events.rs) filters
--      event_type IN ('input.message', 'output.message.completed', 'tool.completed')
--    but the partial index `idx_events_messages` only covered the first two
--    types. Postgres can only use a partial index when the query predicate is
--    *implied* by the index predicate; since the query also matches
--    'tool.completed', the planner fell back to `idx_events_session_sequence`
--    and scanned every event type in the session (deltas, turn.*, tool.started,
--    ...) just to filter three. Widen the partial predicate to match the query.
--
-- 2. The full-text GIN index indexed streaming `output.message.delta` events.
--    Deltas are the highest-volume event type and each one paid GIN maintenance
--    on insert. The final text is already indexed via
--    'output.message.completed', so indexing partial streaming chunks adds write
--    cost and noisy partial matches with no search value. Drop 'delta' from the
--    GIN predicate. (The generated `search_vector` column is unchanged; only its
--    index footprint shrinks. Skipping tsvector *generation* for deltas needs a
--    column rewrite and is left as a follow-up.)
--
-- 3. Two redundant indexes added write cost for no read benefit:
--    - `idx_events_session_id` is a strict prefix of
--      `idx_events_session_sequence(session_id, sequence)`, which also serves the
--      ON DELETE CASCADE FK lookup. Fully redundant.
--    - `idx_events_event_type` is a single low-selectivity column; every observed
--      query that filters event_type also scopes by session_id (and is served by
--      the session-scoped partial indexes), so the planner has no reason to pick
--      a global event_type scan.

-- 1. Recreate the message partial index so it matches the per-turn read query.
DROP INDEX IF EXISTS idx_events_messages;
CREATE INDEX idx_events_messages ON events(session_id, sequence)
    WHERE event_type IN ('input.message', 'output.message.completed', 'tool.completed');
COMMENT ON INDEX idx_events_messages IS
    'Partial index for message reads (input.message, output.message.completed, tool.completed)';

-- 2. Narrow the search GIN index to drop high-volume streaming deltas.
DROP INDEX IF EXISTS idx_events_search_vector;
CREATE INDEX idx_events_search_vector
    ON events USING GIN (search_vector)
    WHERE event_type IN (
        'input.message',
        'output.message.completed',
        'tool.completed'
    );

-- 3. Drop redundant / low-value indexes to cut per-insert write amplification.
DROP INDEX IF EXISTS idx_events_session_id;
DROP INDEX IF EXISTS idx_events_event_type;
