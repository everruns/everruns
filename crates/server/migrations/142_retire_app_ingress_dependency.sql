-- Freeze the App record as archival data while preserving permanent ingress aliases.
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
