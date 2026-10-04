-- Carry per-organization opt-ins across the `app_budgets` -> `endpoint_budgets`
-- feature flag rename. `app_budgets` is an Adoption-grade flag, so in prod an org
-- sees budgets only through a row here; renaming the flag in code without moving
-- the rows would silently switch the budgets UI off for every org that opted in.
--
-- Keep an existing `endpoint_budgets` row if one somehow exists (it is the newer
-- name, so it wins), and drop the stale `app_budgets` rows either way: no code
-- reads that name after this migration.
INSERT INTO org_feature_flags (org_id, flag_name, enabled, created_at, updated_at)
SELECT org_id, 'endpoint_budgets', enabled, created_at, NOW()
FROM org_feature_flags
WHERE flag_name = 'app_budgets'
ON CONFLICT (org_id, flag_name) DO NOTHING;

DELETE FROM org_feature_flags WHERE flag_name = 'app_budgets';
