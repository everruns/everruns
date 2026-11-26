-- Remove the retired App aggregate. Channel/trigger rows, public IDs, ownership,
-- encrypted configuration, conversation bindings and immutable events stay intact.
-- This is a contract migration: stop old API/worker processes before applying it.

-- Complete structural attribution before dropping the archival FK. Only a unique,
-- same-tenant channel named by a server-reserved tag can be adopted.
WITH candidates AS (
    SELECT s.id, min(c.id::text)::uuid AS channel_id
    FROM sessions s
    JOIN agent_channels c ON c.app_id = s.app_id
    JOIN agents a ON a.id = c.agent_id AND a.org_id = s.org_id
    WHERE s.channel_id IS NULL AND s.trigger_id IS NULL
      AND (s.agent_id IS NULL OR s.agent_id = c.agent_id)
      AND (s.tags @> ARRAY['app_channel:' || c.public_id]
        OR s.tags @> ARRAY['slack:endpoint:' || c.public_id]
        OR s.tags @> ARRAY['fcp:endpoint:' || c.public_id])
    GROUP BY s.id HAVING count(*) = 1
)
UPDATE sessions s SET channel_id = c.channel_id
FROM candidates c WHERE c.id = s.id;

-- Live routing tags now name the channel. Unknown/unattributed historical tags
-- are retained as history rather than guessed into a different conversation.
UPDATE sessions s SET tags = ARRAY(
    SELECT DISTINCT CASE
        WHEN starts_with(t, 'app:') THEN 'ingress:' || c.public_id
        WHEN starts_with(t, 'app_channel:') THEN 'channel:' || c.public_id
        WHEN starts_with(t, 'app_channel_type:') THEN replace(t, 'app_channel_type:', 'channel_type:')
        WHEN starts_with(t, 'app_invocation:') THEN replace(t, 'app_invocation:', 'channel_invocation:')
        WHEN t = '__internal:app_invocation' THEN '__internal:channel_invocation'
        ELSE t END
    FROM unnest(s.tags) t
)
FROM agent_channels c WHERE s.channel_id = c.id;

-- Migrated webhook triggers keep their stored execution owner/harness/identity.
UPDATE sessions s SET tags = ARRAY(
    SELECT DISTINCT t FROM unnest(s.tags || ARRAY[
        'agent_trigger:' || 'trg_' || replace(s.trigger_id::text, '-', ''), '__internal:agent_trigger'
    ]) t
) WHERE s.trigger_id IS NOT NULL;

UPDATE durable_schedules SET target_name = 'invoke_scheduled_channel',
    target_input = target_input - 'app_id'
WHERE target_name = 'invoke_scheduled_app_channel';
UPDATE durable_task_queue SET activity_type = 'invoke_scheduled_channel', input = input - 'app_id'
WHERE activity_type = 'invoke_scheduled_app_channel';

-- Unsupported retired eval targets remain readable as historical external labels;
-- the original target is retained outside the typed API shape, never executed
-- or exposed as public External.params (which could leak internal identifiers).
CREATE FUNCTION pg_temp.retire_eval_target(value jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN value->>'type' = 'app'
      THEN jsonb_build_object('type', 'external', 'provider', 'everruns',
        'model', 'retired-target', 'retired_target', value)
      ELSE value END
$$;
UPDATE evals SET target = pg_temp.retire_eval_target(target) WHERE target->>'type' = 'app';
UPDATE eval_cases SET target = pg_temp.retire_eval_target(target) WHERE target->>'type' = 'app';
UPDATE eval_runs SET target = pg_temp.retire_eval_target(target) WHERE target->>'type' = 'app';
UPDATE eval_case_results SET target = pg_temp.retire_eval_target(target) WHERE target->>'type' = 'app';
UPDATE eval_case_results SET target_snapshot = pg_temp.retire_eval_target(target_snapshot) WHERE target_snapshot->>'type' = 'app';
UPDATE eval_case_proposals SET "case" = jsonb_set("case", '{target}', pg_temp.retire_eval_target("case"->'target')) WHERE "case"->'target'->>'type' = 'app';

DROP VIEW app_channels;
ALTER TABLE sessions DROP COLUMN app_id;
ALTER TABLE agent_channels DROP COLUMN app_id, DROP COLUMN legacy_alias_id;
ALTER TABLE agent_triggers DROP COLUMN execution_app_id, DROP COLUMN legacy_alias_id,
    DROP COLUMN legacy_alias_name;
ALTER TABLE IF EXISTS fact_budget_posting DROP COLUMN IF EXISTS app_id;
ALTER TABLE IF EXISTS fact_capability_usage DROP COLUMN IF EXISTS app_id;
ALTER TABLE IF EXISTS fact_llm_generation DROP COLUMN IF EXISTS app_id;
ALTER TABLE IF EXISTS fact_session DROP COLUMN IF EXISTS app_id;
ALTER TABLE IF EXISTS fact_tool_call DROP COLUMN IF EXISTS app_id;
ALTER TABLE IF EXISTS fact_turn DROP COLUMN IF EXISTS app_id;
DROP TABLE apps;
COMMENT ON TABLE agent_channels IS 'Agent-owned communication channels.';
COMMENT ON COLUMN sessions.channel_id IS 'Channel whose ingress created this session.';
