-- Add model_profile column to llm_models table
-- This stores model-specific settings like supported reasoning levels

ALTER TABLE llm_models
ADD COLUMN model_profile JSONB NOT NULL DEFAULT '{}';

-- Example model_profile structure:
-- {
--   "reasoning_effort": {
--     "supported": true,
--     "levels": [
--       {"value": "low", "label": "Low", "description": "Faster responses, minimal reasoning"},
--       {"value": "medium", "label": "Medium", "description": "Balanced depth and efficiency (default)"},
--       {"value": "high", "label": "High", "description": "Deeper reasoning, more detailed explanations"}
--     ],
--     "default": "medium"
--   }
-- }

-- Add comment explaining the column
COMMENT ON COLUMN llm_models.model_profile IS 'Model-specific configuration like reasoning effort levels, extended thinking support, etc.';
