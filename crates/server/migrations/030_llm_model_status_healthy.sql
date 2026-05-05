-- Rename the LLM model status value `active` to `healthy` to match the UI's
-- green-dot indicator that the model is "configured and ready for use".
-- The set of allowed values is otherwise unchanged.
--
-- Provider status (`llm_providers.status`) is intentionally left as `active`;
-- only the model-level status is renamed here.

ALTER TABLE llm_models DROP CONSTRAINT IF EXISTS llm_models_status_check;

UPDATE llm_models SET status = 'healthy' WHERE status = 'active';

ALTER TABLE llm_models
    ALTER COLUMN status SET DEFAULT 'healthy';

ALTER TABLE llm_models
    ADD CONSTRAINT llm_models_status_check
    CHECK (status IN ('healthy', 'disabled'));
