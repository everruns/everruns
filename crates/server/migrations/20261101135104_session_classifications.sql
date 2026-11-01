-- When each session was last read by the built-in session classifiers (Evals
-- rebuild, detection layer 2). One row per session, updated in place: the
-- claim keeps replicas from classifying the same session twice, and the
-- answers say what the classifiers concluded. Findings themselves are health
-- issues; nothing here copies session content.
CREATE TABLE session_classifications (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    -- Why it was read: 'flagged' by a layer 1 detector, or 'sampled'.
    reason TEXT NOT NULL CHECK (reason IN ('flagged', 'sampled')),
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    classified_at TIMESTAMPTZ,
    -- Question id -> probability of yes, plus the answering model.
    answers JSONB,
    error TEXT
);

CREATE INDEX session_classifications_org_idx ON session_classifications (org_id);
