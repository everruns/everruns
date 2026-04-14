-- Rename installed -> enabled on llm_models
-- The "installed" terminology implied downloading a binary; "enabled" better
-- describes the visibility toggle semantics.

ALTER TABLE llm_models RENAME COLUMN installed TO enabled;

-- Recreate the partial index with the new column name
DROP INDEX IF EXISTS idx_llm_models_installed;
CREATE INDEX idx_llm_models_enabled ON llm_models(enabled) WHERE enabled = TRUE;
