-- Observers — online scoring of production sessions. See knowledge/evaluation/online-evals.md.
--
-- trace_scores doubles as the scoring queue: matching inserts 'pending' rows,
-- workers claim them (FOR UPDATE SKIP LOCKED) and write results in place.
-- The unique (observer_id, scorer_key, session_id, turn_id) key makes
-- enqueueing idempotent under at-least-once event delivery.

CREATE TABLE observers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    match_config JSONB NOT NULL DEFAULT '{}',
    sampling_rate DOUBLE PRECISION NOT NULL DEFAULT 0.1
        CHECK (sampling_rate >= 0.0 AND sampling_rate <= 1.0),
    scorers JSONB NOT NULL DEFAULT '[]',
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'paused', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    UNIQUE(org_id, public_id),
    CONSTRAINT observers_public_id_format CHECK (public_id ~ '^observer_[0-9a-f]{32}$'),
    CONSTRAINT observers_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id)
);

CREATE INDEX idx_observers_org_id ON observers(org_id);
CREATE INDEX idx_observers_public_id ON observers(public_id);
-- Matching runs on every turn.completed: keep the active-observer lookup cheap.
CREATE INDEX idx_observers_org_active ON observers(org_id) WHERE status = 'active';

CREATE TRIGGER update_observers_updated_at
    BEFORE UPDATE ON observers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE trace_scores (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    observer_id UUID NOT NULL,
    scorer_key TEXT NOT NULL,
    session_id UUID NOT NULL,
    turn_id TEXT NOT NULL,
    agent_id UUID,
    agent_version_id UUID,
    harness_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'scoring', 'completed', 'errored', 'skipped')),
    attempts INTEGER NOT NULL DEFAULT 0,
    pass BOOLEAN,
    value DOUBLE PRECISION,
    reason TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, public_id),
    UNIQUE(observer_id, scorer_key, session_id, turn_id),
    CONSTRAINT trace_scores_public_id_format CHECK (public_id ~ '^score_[0-9a-f]{32}$'),
    CONSTRAINT trace_scores_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id),
    CONSTRAINT trace_scores_observer_id_fk FOREIGN KEY (observer_id) REFERENCES observers(id) ON DELETE CASCADE,
    CONSTRAINT trace_scores_session_id_fk FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX idx_trace_scores_observer_created ON trace_scores(observer_id, created_at DESC);
CREATE INDEX idx_trace_scores_session_id ON trace_scores(session_id);
-- Worker claim scan: only queue-state rows.
CREATE INDEX idx_trace_scores_queue ON trace_scores(created_at)
    WHERE status IN ('pending', 'scoring');

CREATE TRIGGER update_trace_scores_updated_at
    BEFORE UPDATE ON trace_scores
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
