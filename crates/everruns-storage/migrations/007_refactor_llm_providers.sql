-- Migration 006: Refactor LLM Providers
--
-- Changes:
-- 1. Remove 'ollama' and 'custom' provider types (only openai, anthropic, azure_openai supported)
-- 2. Add 'settings' JSONB column for provider-specific configuration
--
-- Note: This migration will fail if there are existing providers with type 'ollama' or 'custom'.
-- Those must be migrated or deleted before running this migration.

-- First, delete any existing providers with deprecated types (for clean deployments)
-- In production, you might want to migrate these instead
DELETE FROM llm_providers WHERE provider_type IN ('ollama', 'custom');

-- Drop the existing constraint and create a new one with only supported types
ALTER TABLE llm_providers DROP CONSTRAINT IF EXISTS llm_providers_provider_type_check;
ALTER TABLE llm_providers ADD CONSTRAINT llm_providers_provider_type_check
    CHECK (provider_type IN ('openai', 'anthropic', 'azure_openai'));

-- Add settings column for provider-specific configuration
-- Examples:
-- - Azure OpenAI: {"deployment_name": "my-deployment", "api_version": "2024-02-15-preview"}
-- - Custom base_url configs, timeout settings, etc.
ALTER TABLE llm_providers ADD COLUMN IF NOT EXISTS settings JSONB NOT NULL DEFAULT '{}'::jsonb;

-- Add comment for documentation
COMMENT ON COLUMN llm_providers.settings IS 'Provider-specific settings as JSON. E.g., Azure deployment_name, api_version, etc.';
