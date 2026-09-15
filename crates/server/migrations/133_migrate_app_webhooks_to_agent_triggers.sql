-- Move App-owned webhook channels to agent triggers without changing their URLs.

ALTER TABLE agent_triggers
    ADD COLUMN ingress_id VARCHAR(100),
    ADD COLUMN config_encrypted BYTEA;

CREATE UNIQUE INDEX agent_triggers_ingress_id_idx
    ON agent_triggers (ingress_id)
    WHERE ingress_id IS NOT NULL;

CREATE TEMPORARY TABLE migrated_app_webhook_triggers ON COMMIT DROP AS
SELECT
    uuidv7() AS trigger_id,
    apps.id AS app_id,
    app_channels.id AS channel_id,
    app_channels.public_id AS ingress_id,
    apps.org_id,
    agents.id AS agent_id,
    app_channels.channel_config AS config,
    app_channels.channel_config_encrypted AS config_encrypted,
    app_channels.enabled,
    apps.harness_id AS execution_harness_id,
    apps.owner_principal_id AS execution_owner_principal_id,
    apps.resolved_owner_user_id AS execution_resolved_owner_user_id,
    apps.agent_identity_id AS execution_agent_identity_id
FROM app_channels
JOIN apps ON apps.id = app_channels.app_id
JOIN agents ON agents.id = apps.agent_id
WHERE app_channels.channel_type = 'webhook';

INSERT INTO agent_triggers (
    id,
    org_id,
    agent_id,
    trigger_type,
    ingress_id,
    config,
    config_encrypted,
    enabled,
    execution_harness_id,
    execution_owner_principal_id,
    execution_resolved_owner_user_id,
    execution_agent_identity_id,
    execution_app_id,
    status
)
SELECT
    trigger_id,
    org_id,
    agent_id,
    'webhook',
    ingress_id,
    config,
    config_encrypted,
    enabled,
    execution_harness_id,
    execution_owner_principal_id,
    execution_resolved_owner_user_id,
    execution_agent_identity_id,
    app_id,
    'active'
FROM migrated_app_webhook_triggers;

DELETE FROM app_channels AS channel
USING migrated_app_webhook_triggers AS migrated
WHERE channel.id = migrated.channel_id;

UPDATE apps AS app
SET channel_type = NULL, channel_config = '{}'::jsonb, channel_config_encrypted = NULL
FROM migrated_app_webhook_triggers AS migrated
WHERE app.id = migrated.app_id AND app.channel_type = 'webhook';
