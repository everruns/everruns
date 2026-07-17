-- Move App-owned schedule channels to their agent-owned trigger equivalent.
--
-- The existing durable schedule row is retained and retargeted so its execution
-- history, next occurrence, and exactly-once scheduler identity remain intact.
-- Schedule channels that were not actively bound (for example, a draft App or
-- disabled channel) become disabled triggers and can be enabled explicitly.

CREATE TEMPORARY TABLE migrated_app_schedule_triggers ON COMMIT DROP AS
SELECT
    uuidv7() AS trigger_id,
    apps.id AS app_id,
    app_channels.id AS channel_id,
    app_channels.durable_schedule_id,
    apps.org_id,
    apps.agent_id,
    agents.public_id AS agent_public_id,
    app_channels.channel_config AS config,
    COALESCE(durable_schedules.enabled, false) AS enabled
FROM app_channels
JOIN apps ON apps.id = app_channels.app_id
JOIN agents ON agents.id = apps.agent_id
LEFT JOIN durable_schedules ON durable_schedules.id = app_channels.durable_schedule_id
WHERE app_channels.channel_type = 'schedule';

INSERT INTO agent_triggers (
    id,
    org_id,
    agent_id,
    trigger_type,
    config,
    enabled,
    durable_schedule_id,
    status
)
SELECT
    trigger_id,
    org_id,
    agent_id,
    'schedule',
    config,
    enabled,
    durable_schedule_id,
    'active'
FROM migrated_app_schedule_triggers;

UPDATE durable_schedules AS schedule
SET
    name = 'agent-trigger-trg_' || replace(migrated.trigger_id::text, '-', ''),
    description = 'Migrated App schedule trigger trg_' || replace(migrated.trigger_id::text, '-', ''),
    target_type = 'activity',
    target_name = 'invoke_agent_trigger',
    target_input = jsonb_build_object(
        'org_id', migrated.org_id,
        'agent_id', migrated.agent_public_id,
        'trigger_id', 'trg_' || replace(migrated.trigger_id::text, '-', '')
    ),
    updated_at = NOW()
FROM migrated_app_schedule_triggers AS migrated
WHERE schedule.id = migrated.durable_schedule_id;

DELETE FROM app_channels AS channel
USING migrated_app_schedule_triggers AS migrated
WHERE channel.id = migrated.channel_id;

UPDATE apps AS app
SET channel_type = NULL, channel_config = '{}'::jsonb, channel_config_encrypted = NULL
FROM migrated_app_schedule_triggers AS migrated
WHERE app.id = migrated.app_id AND app.channel_type = 'schedule';
