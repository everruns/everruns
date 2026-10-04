-- Carry per-organization opt-ins across the `endpoint_budgets` -> `channel_budgets`
-- feature flag rename. Same reason as 168: the flag is Adoption-grade, so in prod an
-- org sees budgets only through a row here, and renaming the flag in code without
-- moving the rows would silently switch the budgets UI off for every org that opted in.
--
-- 168 renamed `app_budgets` -> `endpoint_budgets` and deleted the `app_budgets` rows,
-- and migrations run in order, so only `endpoint_budgets` can be present here. The
-- Endpoint name was wrong on arrival: management terminology is Channel
-- (knowledge/integrations/agent-exposure.md), and the budget subject type this flag
-- gates is `agent_channel`.
--
-- Keep an existing `channel_budgets` row if one somehow exists (it is the newer name,
-- so it wins), and drop the stale `endpoint_budgets` rows either way: no code reads
-- that name after this migration.
INSERT INTO org_feature_flags (org_id, flag_name, enabled, created_at, updated_at)
SELECT org_id, 'channel_budgets', enabled, created_at, NOW()
FROM org_feature_flags
WHERE flag_name = 'endpoint_budgets'
ON CONFLICT (org_id, flag_name) DO NOTHING;

DELETE FROM org_feature_flags WHERE flag_name = 'endpoint_budgets';
