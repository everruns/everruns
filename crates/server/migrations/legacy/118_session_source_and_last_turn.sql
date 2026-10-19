-- Record how a session started, and give the list a failure signal (EVE-852).
--
-- Two additive columns on `sessions`:
--
--   `source`            — closed set describing the ingress that created the
--                         session. The sessions facet rail enumerates it, so it
--                         is CHECK-constrained rather than free-form.
--   `last_turn_*`       — outcome of the most recent terminal turn. `status`
--                         alone has no notion of failure, so "failed" in the
--                         list and the masthead is derived from these.
--
-- The `last_turn_*` counters follow the incremental-trigger pattern established
-- for `turn_count`/`tool_call_count` in 102_session_reporting_event_counts.sql:
-- statement-level triggers on `events` keep O(1) fields current instead of the
-- list query rescanning event history per row.

ALTER TABLE sessions
    ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN last_turn_status TEXT,
    ADD COLUMN last_turn_at TIMESTAMPTZ,
    ADD COLUMN last_turn_sequence INTEGER;

ALTER TABLE sessions
    ADD CONSTRAINT sessions_source_check CHECK (source IN (
        'chat', 'api', 'slack', 'ag_ui', 'fcp', 'schedule',
        'webhook', 'a2a', 'eval', 'subagent', 'unknown'
    ));

COMMENT ON COLUMN sessions.source IS
    'How the session was started; server-owned for every ingress path (EVE-852)';
COMMENT ON COLUMN sessions.last_turn_status IS
    'Denormalized: status of the highest-sequence turn.completed/failed/cancelled event';

-- ---------------------------------------------------------------------------
-- Backfill `source` with the best inference available; everything else stays
-- explicitly 'unknown' rather than being guessed into a real facet.
-- ---------------------------------------------------------------------------

-- Subagent/delegated children are identifiable from the parent pointer.
UPDATE sessions SET source = 'subagent' WHERE parent_session_id IS NOT NULL;

-- App-channel ingress: the app's channel type is the origin.
UPDATE sessions s
SET source = CASE a.channel_type
        WHEN 'slack' THEN 'slack'
        WHEN 'ag_ui' THEN 'ag_ui'
        WHEN 'fcp' THEN 'fcp'
        WHEN 'schedule' THEN 'schedule'
        WHEN 'webhook' THEN 'webhook'
        WHEN 'api_endpoint' THEN 'webhook'
        WHEN 'a2a' THEN 'a2a'
        WHEN 'public_chat' THEN 'chat'
        ELSE 'unknown'
    END
FROM apps a
WHERE a.id = s.app_id
  AND a.org_id = s.org_id
  AND s.source = 'unknown';

-- Tag conventions used by the two remaining server-owned creators.
UPDATE sessions SET source = 'eval'
 WHERE source = 'unknown' AND tags @> ARRAY['eval'];
UPDATE sessions SET source = 'chat'
 WHERE source = 'unknown' AND tags @> ARRAY['global-chat'];

-- ---------------------------------------------------------------------------
-- Backfill and maintain `last_turn_*`.
-- ---------------------------------------------------------------------------

UPDATE sessions s
SET last_turn_status = CASE last_turn.event_type
        WHEN 'turn.completed' THEN 'completed'
        WHEN 'turn.failed' THEN 'failed'
        WHEN 'turn.cancelled' THEN 'cancelled'
    END,
    last_turn_at = last_turn.ts,
    last_turn_sequence = last_turn.sequence
FROM (
    SELECT DISTINCT ON (e.session_id)
        e.session_id, e.event_type, e.ts, e.sequence
    FROM events e
    WHERE e.event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
    ORDER BY e.session_id, e.sequence DESC
) last_turn
WHERE s.id = last_turn.session_id;

CREATE OR REPLACE FUNCTION sessions_last_turn_insert()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET last_turn_status = CASE delta.event_type
            WHEN 'turn.completed' THEN 'completed'
            WHEN 'turn.failed' THEN 'failed'
            WHEN 'turn.cancelled' THEN 'cancelled'
        END,
        last_turn_at = delta.ts,
        last_turn_sequence = delta.sequence
    FROM (
        SELECT DISTINCT ON (session_id) session_id, event_type, ts, sequence
        FROM new_rows
        WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
        ORDER BY session_id, sequence DESC
    ) delta
    WHERE s.id = delta.session_id
      -- Events are append-only, so a higher sequence is strictly newer. The
      -- guard keeps an out-of-order replay from moving the pointer backwards.
      AND (s.last_turn_sequence IS NULL OR delta.sequence > s.last_turn_sequence);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- Deletion (history trimming, compaction) can remove the turn the pointer
-- refers to, so recompute for the affected sessions. Bounded by
-- idx_events_turns; sessions deleted outright are gone by cascade and skipped.
CREATE OR REPLACE FUNCTION sessions_last_turn_delete()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET last_turn_status = CASE last_turn.event_type
            WHEN 'turn.completed' THEN 'completed'
            WHEN 'turn.failed' THEN 'failed'
            WHEN 'turn.cancelled' THEN 'cancelled'
        END,
        last_turn_at = last_turn.ts,
        last_turn_sequence = last_turn.sequence
    FROM (
        SELECT affected.session_id, recomputed.event_type, recomputed.ts, recomputed.sequence
        FROM (SELECT DISTINCT session_id FROM old_rows
               WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')) affected
        LEFT JOIN LATERAL (
            SELECT e.event_type, e.ts, e.sequence
            FROM events e
            WHERE e.session_id = affected.session_id
              AND e.event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ORDER BY e.sequence DESC
            LIMIT 1
        ) recomputed ON TRUE
    ) last_turn
    WHERE s.id = last_turn.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_last_turn_insert
    AFTER INSERT ON events
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_last_turn_insert();

CREATE TRIGGER sessions_last_turn_delete
    AFTER DELETE ON events
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_last_turn_delete();

-- ---------------------------------------------------------------------------
-- Indexes carrying the new list/facet predicates.
--
-- The list and the facet aggregates share one predicate shape:
--   org_id = $1 [AND source = ...] [AND agent_id = ...]
--            [AND resolved_owner_user_id = ...] [AND created_at BETWEEN ...]
-- ordered by created_at DESC or updated_at DESC. 097 already covers
-- (org_id, created_at DESC); these extend it so each added filter still starts
-- from an org-scoped index scan rather than a sequential scan.
-- ---------------------------------------------------------------------------

CREATE INDEX idx_sessions_org_source_created_at
    ON sessions (org_id, source, created_at DESC);

CREATE INDEX idx_sessions_org_owner_created_at
    ON sessions (org_id, resolved_owner_user_id, created_at DESC);

-- The chat thread list is this endpoint with source=chat + mine, ordered by
-- last activity, so it gets a covering ordering index of its own.
CREATE INDEX idx_sessions_org_source_owner_updated_at
    ON sessions (org_id, source, resolved_owner_user_id, updated_at DESC);

CREATE INDEX idx_sessions_org_updated_at
    ON sessions (org_id, updated_at DESC);

-- Activity derivation reads status + last_turn_status together.
CREATE INDEX idx_sessions_org_status_last_turn
    ON sessions (org_id, status, last_turn_status);
