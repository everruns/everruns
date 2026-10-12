-- drop_narrow_personal_access_tokens
--
-- Personal access token scopes are enforced (knowledge/security/pat-scopes.md).
-- Rows minted before EVE-701 may store scopes other than exactly ["*"]. Nobody
-- chose those with enforcement in mind, so they are neither honored as `read`
-- nor widened to `*`: they are deleted, and their owners mint a new token
-- (owner decision, 2026-10-12). Tokens have no dependent rows.
DELETE FROM personal_access_tokens
WHERE scopes IS DISTINCT FROM '["*"]'::jsonb;
