-- A2A push-notification configs (A2A 1.0 §3.1.7-3.1.10, §4.3.3).
--
-- An A2A task is an endpoint session (task id == session id), so a config
-- hangs off the session and is dropped with it. `org_id` scopes delivery-time
-- reads; the endpoint handler already fenced the session to the calling
-- channel (TM-A2A-012) before writing. `config_id` is the A2A
-- `TaskPushNotificationConfig.id`, unique per task.
--
-- The client's token and credentials are secrets the receiver checks, so they
-- are stored encrypted in one blob and never echoed back. `wire_version` is the
-- A2A version the config was created under, which shapes the payload.

CREATE TABLE a2a_push_configs (
    id                UUID PRIMARY KEY,
    org_id            BIGINT NOT NULL,
    session_id        UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    config_id         TEXT NOT NULL,
    url               TEXT NOT NULL,
    auth_scheme       TEXT,
    secrets_encrypted BYTEA,
    wire_version      TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, config_id)
);
