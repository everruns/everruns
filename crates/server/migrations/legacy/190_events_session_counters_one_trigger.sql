-- One statement-level trigger keeps every sessions counter that events feed.
--
-- Three AFTER INSERT triggers on events each updated the event's sessions row:
-- event_count (125), turn_count/tool_call_count (102) and the last_turn_*
-- pointer (118). Every insert statement therefore wrote the same sessions row
-- three times. A turn writes about ten insert statements, so that was some
-- thirty sessions updates a turn, a third of the database time the turn
-- latency bench measures at eight concurrent sessions. This trigger computes
-- all of them from the transition table and updates each session once.
--
-- The delete triggers stay as they are: deletes are rare (history trimming,
-- compaction) and the last-turn recompute differs from the insert path.

DROP TRIGGER IF EXISTS sessions_event_count_insert ON events;
DROP TRIGGER IF EXISTS sessions_reporting_event_counts_insert ON events;
DROP TRIGGER IF EXISTS sessions_last_turn_insert ON events;
DROP FUNCTION IF EXISTS sessions_event_count_insert();
DROP FUNCTION IF EXISTS sessions_reporting_event_counts_insert();
DROP FUNCTION IF EXISTS sessions_last_turn_insert();

CREATE OR REPLACE FUNCTION sessions_event_counters_insert()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET
        event_count = s.event_count + delta.event_count,
        turn_count = s.turn_count + delta.turn_count,
        tool_call_count = s.tool_call_count + delta.tool_call_count,
        -- Same guard as migration 118: events are append-only, so a higher
        -- sequence is strictly newer, and an out-of-order replay never moves
        -- the pointer backwards.
        last_turn_status = CASE
            WHEN delta.last_turn_sequence IS NOT NULL
             AND (s.last_turn_sequence IS NULL OR delta.last_turn_sequence > s.last_turn_sequence)
            THEN CASE delta.last_turn_type
                WHEN 'turn.completed' THEN 'completed'
                WHEN 'turn.failed' THEN 'failed'
                WHEN 'turn.cancelled' THEN 'cancelled'
            END
            ELSE s.last_turn_status
        END,
        last_turn_at = CASE
            WHEN delta.last_turn_sequence IS NOT NULL
             AND (s.last_turn_sequence IS NULL OR delta.last_turn_sequence > s.last_turn_sequence)
            THEN delta.last_turn_ts
            ELSE s.last_turn_at
        END,
        last_turn_sequence = CASE
            WHEN delta.last_turn_sequence IS NOT NULL
             AND (s.last_turn_sequence IS NULL OR delta.last_turn_sequence > s.last_turn_sequence)
            THEN delta.last_turn_sequence
            ELSE s.last_turn_sequence
        END
    FROM (
        SELECT
            session_id,
            count(*) AS event_count,
            count(*) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ) AS turn_count,
            count(*) FILTER (WHERE event_type = 'tool.completed') AS tool_call_count,
            max(sequence) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ) AS last_turn_sequence,
            (array_agg(event_type ORDER BY sequence DESC) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ))[1] AS last_turn_type,
            (array_agg(ts ORDER BY sequence DESC) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ))[1] AS last_turn_ts
        FROM new_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_event_counters_insert
    AFTER INSERT ON events
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_event_counters_insert();
