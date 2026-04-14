-- API keys: user-scoped (remove org binding)
--
-- API keys become personal, user-scoped tokens. Organization context is now
-- resolved per-request via cookie/header, same as session (JWT) auth.
-- This aligns API key auth with session auth: both produce the same AuthUser
-- shape and use the same org resolution mechanism.

DROP INDEX IF EXISTS idx_api_keys_org_id;
ALTER TABLE api_keys DROP COLUMN org_id;
