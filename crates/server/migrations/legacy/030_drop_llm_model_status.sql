-- Drop the stored `llm_models.status` column.
--
-- The previous design carried two overlapping flags on each model row:
--   * `enabled` — user toggle controlling whether the model shows up in
--                 UI pickers (kept).
--   * `status`  — text column with values `active` / `disabled` that was
--                 set on creation and never meaningfully transitioned;
--                 effectively a soft-delete flag that duplicated `enabled`.
--
-- "Is this model usable right now?" is now derived at read time from the
-- joined provider (`api_key_set` and `status`), exposed as the `healthy`
-- boolean on `LlmModelWithProvider`. There is no need to persist it.

ALTER TABLE llm_models DROP CONSTRAINT IF EXISTS llm_models_status_check;
DROP INDEX IF EXISTS idx_llm_models_status;
ALTER TABLE llm_models DROP COLUMN IF EXISTS status;
