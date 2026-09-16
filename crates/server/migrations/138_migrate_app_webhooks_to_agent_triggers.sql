-- Move App-owned webhook channels to agent triggers without changing their URLs.
LOCK TABLE apps IN SHARE ROW EXCLUSIVE MODE;
LOCK TABLE agent_endpoints IN SHARE ROW EXCLUSIVE MODE;

ALTER TABLE agent_triggers
    ADD COLUMN ingress_id VARCHAR(100),
    ADD COLUMN config_encrypted BYTEA;

CREATE UNIQUE INDEX agent_triggers_ingress_id_idx
    ON agent_triggers (ingress_id)
    WHERE ingress_id IS NOT NULL;
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM agent_endpoints AS endpoint
        JOIN apps AS app ON app.id = endpoint.app_id
        WHERE endpoint.channel_type = 'webhook'
          AND (endpoint.agent_id IS NULL OR app.agent_id IS NULL)
    ) THEN
        RAISE EXCEPTION 'webhook migration requires every App to have an Agent; migration 134 must run first';
    END IF;
END;
$$;

CREATE TEMPORARY TABLE migrated_app_webhook_triggers ON COMMIT DROP AS
SELECT
    uuidv7() AS trigger_id,
    apps.id AS app_id,
    agent_endpoints.id AS endpoint_id,
    agent_endpoints.public_id AS ingress_id,
    apps.org_id,
    agent_endpoints.agent_id,
    agent_endpoints.channel_config AS config,
    agent_endpoints.channel_config_encrypted AS config_encrypted,
    agent_endpoints.status = 'live' AS enabled,
    apps.harness_id AS execution_harness_id,
    agent_endpoints.owner_principal_id AS execution_owner_principal_id,
    agent_endpoints.resolved_owner_user_id AS execution_resolved_owner_user_id,
    agent_endpoints.agent_identity_id AS execution_agent_identity_id
FROM agent_endpoints
JOIN apps ON apps.id = agent_endpoints.app_id
WHERE agent_endpoints.channel_type = 'webhook';

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

DELETE FROM agent_endpoints AS endpoint
USING migrated_app_webhook_triggers AS migrated
WHERE endpoint.id = migrated.endpoint_id;

UPDATE apps AS app
SET channel_type = NULL, channel_config = '{}'::jsonb, channel_config_encrypted = NULL
FROM migrated_app_webhook_triggers AS migrated
WHERE app.id = migrated.app_id AND app.channel_type = 'webhook';
