-- Denormalize session turn/tool counts for fact_session projection (EVE-731).
--
-- `project_session` previously recomputed correlated COUNT(*) subqueries over
-- `events` on every snapshot upsert. As session history grows, each projection
-- update rescanned the full event log. Keep incremental counters on `sessions`
-- maintained by statement-level INSERT/DELETE triggers so repeated projection
-- work reads O(1) fields instead of scanning history.

ALTER TABLE sessions
    ADD COLUMN turn_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN tool_call_count BIGINT NOT NULL DEFAULT 0;

COMMENT ON COLUMN sessions.turn_count IS
    'Denormalized: count of turn.completed, turn.failed, and turn.cancelled events';
COMMENT ON COLUMN sessions.tool_call_count IS
    'Denormalized: count of tool.completed events';

-- One-time backfill from canonical events before triggers take over.
UPDATE sessions s
SET
    turn_count = counts.turn_count,
    tool_call_count = counts.tool_call_count
FROM (
    SELECT
        e.session_id,
        count(*) FILTER (
            WHERE e.event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
        ) AS turn_count,
        count(*) FILTER (WHERE e.event_type = 'tool.completed') AS tool_call_count
    FROM events e
    GROUP BY e.session_id
) counts
WHERE s.id = counts.session_id;

CREATE OR REPLACE FUNCTION sessions_reporting_event_counts_insert()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET
        turn_count = s.turn_count + delta.turn_count,
        tool_call_count = s.tool_call_count + delta.tool_call_count
    FROM (
        SELECT
            session_id,
            count(*) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ) AS turn_count,
            count(*) FILTER (WHERE event_type = 'tool.completed') AS tool_call_count
        FROM new_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id
      AND (delta.turn_count > 0 OR delta.tool_call_count > 0);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION sessions_reporting_event_counts_delete()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET
        turn_count = GREATEST(s.turn_count - delta.turn_count, 0),
        tool_call_count = GREATEST(s.tool_call_count - delta.tool_call_count, 0)
    FROM (
        SELECT
            session_id,
            count(*) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ) AS turn_count,
            count(*) FILTER (WHERE event_type = 'tool.completed') AS tool_call_count
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id
      AND (delta.turn_count > 0 OR delta.tool_call_count > 0);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_reporting_event_counts_insert
    AFTER INSERT ON events
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_reporting_event_counts_insert();

CREATE TRIGGER sessions_reporting_event_counts_delete
    AFTER DELETE ON events
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_reporting_event_counts_delete();
