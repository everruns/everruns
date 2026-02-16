-- Per-user pinned sessions
-- Users can pin sessions for quick access. Pins are scoped per-user per-org.

CREATE TABLE pinned_sessions (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    pinned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, session_id)
);

-- Index for listing a user's pinned sessions in an org
CREATE INDEX idx_pinned_sessions_user_org ON pinned_sessions(user_id, org_id);
