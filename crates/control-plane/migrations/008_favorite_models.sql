-- Migration: Add favorite models support
-- Allows users to mark models as favorites for quick access in the model picker

ALTER TABLE llm_models ADD COLUMN is_favorite BOOLEAN NOT NULL DEFAULT FALSE;

-- Index for efficient favorite model queries
CREATE INDEX idx_llm_models_favorite ON llm_models(is_favorite) WHERE is_favorite = TRUE;

COMMENT ON COLUMN llm_models.is_favorite IS 'Whether this model is marked as a favorite for quick access';
