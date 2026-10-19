-- Take the sessions row off the event insert path.
--
-- Migration 190 folded the insert triggers into one, but every events insert
-- statement still updated the session's row: about ten sessions updates a
-- turn, each taking the row lock that status updates, token totals and other
-- inserts for the same session wait on. Only two things have to stay atomic
-- with an event: the event row and its sequence number. The counters do not.
--
--   * event_count is derived on read. Every event takes the next
--     event_sequences number, so the count is the numbers handed out minus the
--     ones with no live event. event_sequences.removed_count holds the latter:
--     events deleted by retention, plus the rare number handed out to an
--     insert that conflicted and wrote nothing. The bump of event_sequences
--     the insert already does is the only write.
--   * turn_count, tool_call_count and last_turn_* change once per turn, when
--     its terminal event (turn.completed, turn.failed, turn.cancelled) is
--     inserted. A row trigger with a WHEN clause fires only for those events;
--     every other insert evaluates the clause and writes nothing. The turn's
--     tool calls are counted from events between the previous terminal event
--     and this one, over the (session_id, sequence) key.
--
-- Trade-off: tool_call_count now moves at turn end rather than per tool call.
-- Its only reader is reporting, which projects sessions after turns finish. A
-- tool call outside any turn is counted with the next turn that finishes.
--
-- The delete triggers for turn/tool counts (102) and last_turn_* (118) stay.

-- ---------------------------------------------------------------------------
-- event_count -> event_sequences
-- ---------------------------------------------------------------------------

ALTER TABLE event_sequences
    ADD COLUMN removed_count INTEGER NOT NULL DEFAULT 0;

COMMENT ON COLUMN event_sequences.removed_count IS
    'Sequence numbers handed out with no live event (deleted by retention, or a conflicting insert); event count = next_sequence - 1 - removed_count';

-- Keep today's counts exact: whatever the stored counter does not account for
-- is a number with no live event.
UPDATE event_sequences es
SET removed_count = GREATEST(es.next_sequence - 1 - s.event_count, 0)
FROM sessions s
WHERE s.id = es.session_id
  AND es.next_sequence - 1 <> s.event_count;

DROP TRIGGER IF EXISTS sessions_event_count_delete ON events;
DROP FUNCTION IF EXISTS sessions_event_count_delete();

CREATE OR REPLACE FUNCTION event_sequences_removed_count_delete()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE event_sequences es
    SET removed_count = es.removed_count + delta.removed
    FROM (
        SELECT session_id, count(*) AS removed
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE es.session_id = delta.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER event_sequences_removed_count_delete
    AFTER DELETE ON events
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION event_sequences_removed_count_delete();

ALTER TABLE sessions DROP COLUMN event_count;

-- ---------------------------------------------------------------------------
-- turn_count, tool_call_count, last_turn_* -> once per turn
-- ---------------------------------------------------------------------------

DROP TRIGGER IF EXISTS sessions_event_counters_insert ON events;
DROP FUNCTION IF EXISTS sessions_event_counters_insert();

CREATE OR REPLACE FUNCTION sessions_turn_finished()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET
        turn_count = s.turn_count + 1,
        tool_call_count = s.tool_call_count + (
            SELECT count(*)
            FROM events e
            WHERE e.session_id = NEW.session_id
              AND e.sequence > COALESCE(s.last_turn_sequence, 0)
              AND e.sequence < NEW.sequence
              AND e.event_type = 'tool.completed'
        ),
        -- Events are append-only, so a higher sequence is strictly newer, and
        -- an out-of-order replay never moves the pointer backwards (118).
        last_turn_status = CASE
            WHEN s.last_turn_sequence IS NULL OR NEW.sequence > s.last_turn_sequence
            THEN CASE NEW.event_type
                WHEN 'turn.completed' THEN 'completed'
                WHEN 'turn.failed' THEN 'failed'
                WHEN 'turn.cancelled' THEN 'cancelled'
            END
            ELSE s.last_turn_status
        END,
        last_turn_at = CASE
            WHEN s.last_turn_sequence IS NULL OR NEW.sequence > s.last_turn_sequence
            THEN NEW.ts
            ELSE s.last_turn_at
        END,
        last_turn_sequence = GREATEST(s.last_turn_sequence, NEW.sequence)
    WHERE s.id = NEW.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_turn_finished
    AFTER INSERT ON events
    FOR EACH ROW
    WHEN (NEW.event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled'))
    EXECUTE FUNCTION sessions_turn_finished();
