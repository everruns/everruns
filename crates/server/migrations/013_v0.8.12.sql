-- v0.8.12: model visibility + personal API keys
-- Squashed from: 013_rename_installed_to_enabled.sql + 014_api_keys_user_scoped.sql

-- ============================================
-- 013_rename_installed_to_enabled: Rename installed -> enabled on llm_models
-- The "installed" terminology implied downloading a binary; "enabled" better
-- describes the visibility toggle semantics.
-- ============================================

ALTER TABLE llm_models RENAME COLUMN installed TO enabled;

-- Recreate the partial index with the new column name
DROP INDEX IF EXISTS idx_llm_models_installed;
CREATE INDEX IF NOT EXISTS idx_llm_models_enabled ON llm_models(enabled) WHERE enabled = TRUE;

-- ============================================
-- 014_api_keys_user_scoped: API keys become user-scoped
-- Organization context is now resolved per-request via cookie/header, the same
-- way as session auth.
-- ============================================

DROP INDEX IF EXISTS idx_api_keys_org_id;
ALTER TABLE api_keys DROP COLUMN org_id;
