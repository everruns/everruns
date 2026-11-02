-- Observers v2 (Evals rebuild, step 6): session and tool-call scopes, a
-- delayed claim for session scores, a person's review of a score, and a
-- pass-rate alert on the observer.

-- `turn_id` stays the dedupe key: a turn id for turn scope, the literal
-- 'session' for session scope (one score per session, re-armed when the
-- session idles again), the tool call id for tool scope.
ALTER TABLE trace_scores
    ADD COLUMN scope text NOT NULL DEFAULT 'turn',
    ADD COLUMN not_before timestamptz,
    ADD COLUMN review jsonb,
    ADD CONSTRAINT trace_scores_scope_check CHECK (scope IN ('turn', 'session', 'tool'));

ALTER TABLE observers
    ADD COLUMN alert jsonb;

-- Live-quality summaries read completed scores by org and time.
CREATE INDEX idx_trace_scores_org_completed
    ON trace_scores (org_id, updated_at DESC)
    WHERE status = 'completed';
