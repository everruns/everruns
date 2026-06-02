-- Rename api_keys -> personal_access_tokens.
-- These tokens are user-scoped (tied to a person, inheriting all their org
-- access), never org-scoped. "api_keys" was ambiguous against the unrelated
-- LLM-provider / integration / MCP "api key" concepts elsewhere in the schema.
-- The "personal access token" name (GitHub/GitLab convention) makes the
-- user-scoped meaning explicit.

ALTER TABLE api_keys RENAME TO personal_access_tokens;

ALTER TABLE personal_access_tokens RENAME COLUMN key_hash TO token_hash;
ALTER TABLE personal_access_tokens RENAME COLUMN key_prefix TO token_prefix;

ALTER INDEX idx_api_keys_hash RENAME TO idx_personal_access_tokens_hash;
ALTER INDEX idx_api_keys_user_id RENAME TO idx_personal_access_tokens_user_id;
ALTER INDEX idx_api_keys_expires_at RENAME TO idx_personal_access_tokens_expires_at;
