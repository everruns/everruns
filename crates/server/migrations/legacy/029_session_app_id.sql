-- Track durable app provenance for sessions created by app channels.

ALTER TABLE sessions
    ADD COLUMN app_id UUID REFERENCES apps(id) ON DELETE SET NULL;

CREATE INDEX idx_sessions_app_id ON sessions(app_id);

COMMENT ON COLUMN sessions.app_id IS
    'Internal app UUID for sessions created by app channels; NULL for user/API/platform-created sessions.';
