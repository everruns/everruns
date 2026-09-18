-- Freeze the App record as archival data while preserving permanent ingress aliases.
ALTER TABLE agent_triggers
    ADD COLUMN execution_app_public_id TEXT,
    ADD COLUMN execution_app_name TEXT,
    ADD COLUMN execution_agent_version_policy TEXT,
    ADD COLUMN execution_agent_version_id UUID;

UPDATE agent_triggers AS trigger
SET execution_app_public_id = app.public_id,
    execution_app_name = app.name,
    execution_agent_version_policy = app.agent_version_policy,
    execution_agent_version_id = app.agent_version_id
FROM apps AS app
WHERE trigger.execution_app_id = app.id;
ALTER TABLE agent_endpoints
    ADD COLUMN legacy_app_public_id TEXT;

UPDATE agent_endpoints AS endpoint
SET legacy_app_public_id = app.public_id
FROM apps AS app
WHERE app.id = endpoint.app_id;

ALTER TABLE agent_endpoints
    ALTER COLUMN app_id DROP NOT NULL;

CREATE INDEX idx_agent_endpoints_legacy_app_channel_type
    ON agent_endpoints (legacy_app_public_id, channel_type);

COMMENT ON COLUMN agent_endpoints.app_id IS
    'Optional archival foreign key to the App that created this endpoint. Native Agent endpoints leave it null.';
COMMENT ON COLUMN agent_endpoints.legacy_app_public_id IS
    'Immutable alias identity for permanent /v1/apps/{app_id}/... ingress routes. Native Agent endpoints leave it null.';
COMMENT ON COLUMN sessions.app_id IS
    'Archival provenance for sessions created through the retired App model. endpoint_id is the live ingress pointer.';
COMMENT ON COLUMN agent_triggers.execution_app_public_id IS
    'Frozen permanent alias identity for webhook triggers migrated from Apps.';
COMMENT ON COLUMN agent_triggers.execution_app_name IS
    'Frozen App name used by migrated webhook templates and session titles.';
COMMENT ON COLUMN agent_triggers.execution_agent_version_policy IS
    'Frozen App agent-version policy used by migrated trigger sessions.';
COMMENT ON COLUMN agent_triggers.execution_agent_version_id IS
    'Frozen pinned App agent version used by migrated trigger sessions.';

DROP VIEW app_channels;
CREATE VIEW app_channels AS
SELECT
    id,
    app_id,
    public_id,
    channel_type,
    channel_config,
    channel_config_encrypted,
    durable_schedule_id,
    enabled,
    created_at,
    updated_at,
    auth,
    auth_encrypted
FROM agent_endpoints;
