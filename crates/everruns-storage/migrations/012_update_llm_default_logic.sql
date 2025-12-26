-- Migration: Update LLM default logic
-- 1. Remove is_default from llm_providers entirely
-- 2. Change llm_models default to be global (only one default model across all providers)

-- Drop the unique index on providers default
DROP INDEX IF EXISTS idx_llm_providers_default;

-- Remove is_default column from llm_providers
ALTER TABLE llm_providers DROP COLUMN IF EXISTS is_default;

-- Drop the per-provider unique index on model default
DROP INDEX IF EXISTS idx_llm_models_provider_default;

-- Create a new global unique index for model default (only one default model globally)
CREATE UNIQUE INDEX idx_llm_models_default ON llm_models(is_default) WHERE is_default = TRUE;
