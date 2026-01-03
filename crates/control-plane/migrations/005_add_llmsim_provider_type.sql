-- Add 'llmsim' provider type for testing
-- This allows the LlmSimDriver to be used in integration tests

-- Drop the old constraint
ALTER TABLE llm_providers DROP CONSTRAINT IF EXISTS llm_providers_provider_type_check;

-- Add the new constraint with llmsim included
ALTER TABLE llm_providers ADD CONSTRAINT llm_providers_provider_type_check
    CHECK (provider_type IN ('openai', 'anthropic', 'azure_openai', 'llmsim'));
