-- Lift endpoint authentication out of transport-specific channel_config.
ALTER TABLE agent_endpoints
    ADD COLUMN auth JSONB,
    ADD COLUMN auth_encrypted BYTEA;

-- Plaintext rows can move in place. Encrypted legacy rows remain untouched
-- until an application write can decrypt and split them safely.
UPDATE agent_endpoints
SET auth = channel_config -> 'auth',
    channel_config = channel_config - 'auth'
WHERE channel_config_encrypted IS NULL
  AND channel_config ? 'auth';

-- Keep the transitional App compatibility view readable by old and new code.
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
