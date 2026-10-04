CREATE TABLE health_issues (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    episode_id UUID NOT NULL DEFAULT uuidv7(),
    status TEXT NOT NULL CHECK (status IN ('open', 'needs_check', 'resolved', 'inapplicable')),
    missing_scopes TEXT[] NOT NULL DEFAULT '{}',
    error_code TEXT,
    channel_revision TIMESTAMPTZ NOT NULL,
    first_detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_checked_at TIMESTAMPTZ NOT NULL,
    UNIQUE (org_id, channel_id, code)
);
CREATE INDEX health_issues_org_status ON health_issues(org_id, status, first_detected_at DESC);

CREATE TABLE health_issue_reminders (
    issue_id UUID NOT NULL REFERENCES health_issues(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    episode_id UUID NOT NULL,
    snoozed_until TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (issue_id, user_id)
);

-- A health announcement belongs to one episode even after it is read.
CREATE UNIQUE INDEX notifications_health_episode
    ON notifications(org_id, user_id, dedupe_key)
    WHERE kind = 'health.issue';

CREATE FUNCTION remove_health_issue_notifications() RETURNS TRIGGER AS $$
BEGIN
    DELETE FROM notifications
    WHERE org_id = OLD.org_id AND kind = 'health.issue'
      AND target_id = OLD.id::text;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER health_issue_deleted BEFORE DELETE ON health_issues
    FOR EACH ROW EXECUTE FUNCTION remove_health_issue_notifications();
