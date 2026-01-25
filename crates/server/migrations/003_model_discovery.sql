-- Migration: Model Discovery Support
-- Enables dynamic model discovery from provider APIs with sync tracking

-- Source indicates how the model was added to the database
-- - manual: User-created via API or UI
-- - discovered: From provider's list_models API
-- - predefined: From hardcoded seed data
ALTER TABLE llm_models ADD COLUMN source TEXT NOT NULL DEFAULT 'manual'
    CHECK (source IN ('manual', 'discovered', 'predefined'));

-- Last time this model was seen in provider's API response
-- Used to detect stale/removed models
ALTER TABLE llm_models ADD COLUMN last_seen_at TIMESTAMPTZ;

-- Raw metadata from provider's API (for debugging and extended info)
ALTER TABLE llm_models ADD COLUMN provider_metadata JSONB;

-- Track when the provider was last synced
ALTER TABLE llm_providers ADD COLUMN last_synced_at TIMESTAMPTZ;

-- Index for efficient stale model queries
CREATE INDEX idx_llm_models_source ON llm_models(source);
CREATE INDEX idx_llm_models_last_seen ON llm_models(last_seen_at)
    WHERE last_seen_at IS NOT NULL;

COMMENT ON COLUMN llm_models.source IS 'How the model was added: manual, discovered, or predefined';
COMMENT ON COLUMN llm_models.last_seen_at IS 'Last time model was seen in provider API response';
COMMENT ON COLUMN llm_models.provider_metadata IS 'Raw metadata from provider API response';
COMMENT ON COLUMN llm_providers.last_synced_at IS 'Last time models were synced from provider API';
