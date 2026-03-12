-- Generic user notifications and UI inbox support.
-- Design:
-- - Notifications are durable and user-scoped so the bell counter survives refreshes.
-- - Delivery surfaces (UI bell, toast, future email) read from the same canonical record.
-- - Turn-complete notifications resolve recipients through input_message_id -> user_id mapping.

CREATE TABLE notifications (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind VARCHAR(100) NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    target_type VARCHAR(50),
    target_id TEXT,
    href TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    dedupe_key TEXT,
    occurrence_count INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count > 0),
    viewed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notifications_user_created
    ON notifications(user_id, org_id, created_at DESC);
CREATE INDEX idx_notifications_user_updated
    ON notifications(user_id, org_id, updated_at DESC);
CREATE INDEX idx_notifications_unviewed
    ON notifications(user_id, org_id, viewed_at, created_at DESC);
CREATE UNIQUE INDEX idx_notifications_active_dedupe
    ON notifications(org_id, user_id, dedupe_key)
    WHERE dedupe_key IS NOT NULL AND viewed_at IS NULL;

CREATE TRIGGER update_notifications_updated_at
    BEFORE UPDATE ON notifications
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE notification_turn_requests (
    input_message_id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notification_turn_requests_user
    ON notification_turn_requests(user_id, created_at DESC);
CREATE INDEX idx_notification_turn_requests_session
    ON notification_turn_requests(session_id, created_at DESC);

CREATE OR REPLACE FUNCTION notify_notification_available()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify(
        'notification_available',
        json_build_object(
            'org_id', NEW.org_id,
            'user_id', NEW.user_id,
            'action', lower(TG_OP)
        )::text
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notifications_change_notify
    AFTER INSERT OR UPDATE ON notifications
    FOR EACH ROW
    EXECUTE FUNCTION notify_notification_available();
