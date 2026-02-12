-- Remove DB-level provider_type check constraint.
-- Validation is handled in application layer to allow forward-compatible provider types.

ALTER TABLE llm_providers
    DROP CONSTRAINT IF EXISTS llm_providers_provider_type_check;
