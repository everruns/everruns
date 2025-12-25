-- Remove context_window column from llm_models
-- Context window info is now retrieved from ModelProfile

ALTER TABLE llm_models DROP COLUMN context_window;
